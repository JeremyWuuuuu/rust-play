extern crate tokio;
extern crate httparse;
#[macro_use]
extern crate futures;
extern crate bytes;
extern crate http;

use tokio::prelude::*;
use tokio::io::copy;
use tokio::net::{TcpListener, TcpStream, Incoming};
use tokio::codec::{Decoder, Encoder};
use futures::future;
use std::{io, env, fmt};
use bytes::BytesMut;
use http::{Request, Response, StatusCode};
use http::header::HeaderValue;
use httparse::Status;
use std::convert::From;

fn main() {
    // Bind the server's socket.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let listener = TcpListener::bind(&addr)
        .expect("unable to bind TCP listener");
    let server = HttpServer {
        listener,
    }.map_err(|e| {eprintln!("err = {:?}", e)}).and_then(|_| {
        println!("running");
        future::ok(())
    });
    // Pull out a stream of sockets for incoming connections
    // let server = listener.incoming()
    //     .map_err(|e| eprintln!("accept failed = {:?}", e))
    //     .for_each(|sock| {
    //         // Split up the reading and writing parts of the
    //         // socket.
    //         println!("task");
    //         process(sock);
    //         Ok(())
    //     });

    // Start the Tokio runtime
    tokio::run(server);
}

fn process(socket: TcpStream) -> future::FutureResult<(), ()> {
    let (tx, rx) = Http.framed(socket).split();

    println!("{:?}, {:?}", &tx, &rx);
    // let task = tx.send(rx.).then(|res| {
    //     if let Err(e) = res {
    //         println!("failed to process connection; error = {:?}", e);
    //     }
    //     Ok(())
    // });

    // tokio::spawn(task);
    // future::
    // return tx.send_all(rx.and_then(respond)).then(|res| {
    //     Ok(())
    // })

    future::ok(())
}

#[derive(Debug)]
struct Http;


impl Encoder for Http {
    type Item = Response<String>;
    type Error = io::Error;

    fn encode(&mut self, item: Response<String>, dst: &mut BytesMut) -> io::Result<()> {
        use std::fmt::Write;

        write!(
            BytesWrite(dst),
            "\
            HTTP/1.1 {}\r\n\
            Server: Proxy\r\n\
            Content-Length: {}\r\n\
            ",
            item.status(),
            item.body().len()
        ).unwrap();

        for (k, v) in item.headers() {
            dst.extend_from_slice(k.as_str().as_bytes());
            dst.extend_from_slice(b": ");
            dst.extend_from_slice(v.as_bytes());
            dst.extend_from_slice(b"\r\n");
        }

        dst.extend_from_slice(b"\r\n");
        dst.extend_from_slice(item.body().as_bytes());

        return Ok(());

        struct BytesWrite<'a>(&'a mut BytesMut);

        impl<'a> fmt::Write for BytesWrite<'a> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                self.0.extend_from_slice(s.as_bytes());
                Ok(())
            }

            fn write_fmt(&mut self, args: fmt::Arguments) -> fmt::Result {
                fmt::write(self, args)
            }
        }
    }
}

impl Decoder for Http {
    type Item = Request<()>;
    type Error = io::Error;


    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Request<()>>> {
        let mut headers = [None; 16];
        println!("{:p}", src);
        let (method, path, version, amt) = {
            let mut parsed_headers = [httparse::EMPTY_HEADER; 16];
            let mut r = httparse::Request::new(&mut parsed_headers);
            let status = r.parse(src).map_err(|e| {
                let msg = format!("failed to parse http request: {:?}", e);
                io::Error::new(io::ErrorKind::Other, msg)
            })?;

            println!("{:?}", status);

            let amt = match status {
                Status::Complete(amt) => amt,
                _ => return Ok(None),
            };

            let toslice = |a: &[u8]| {
                let start = a.as_ptr() as usize - src.as_ptr() as usize;
                assert!(start < src.len());

                (start, start + a.len())
            };

            for (i, header) in r.headers.iter().enumerate() {
                let k = toslice(header.name.as_bytes());
                let v = toslice(header.value);
                headers[i] = Some((k, v));
            }

            (
                toslice(r.method.unwrap().as_bytes()),
                toslice(r.path.unwrap().as_bytes()),
                r.version.unwrap(),
                amt,
            )
        };

        if version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "only HTTP/1.1 accepted",
            ));
        }
        let data = src.split_to(amt).freeze();
        let mut ret = Request::builder();
        ret.method(&data[method.0..method.1]);
        ret.uri(data.slice(path.0, path.1));
        ret.version(http::Version::HTTP_11);
        for header in headers.iter() {
            let (k, v) = match *header {
                Some((ref k, ref v)) => (k, v),
                None => break,
            };
            let value = unsafe { HeaderValue::from_shared_unchecked(data.slice(v.0, v.1)) };
            ret.header(&data[k.0..k.1], value);
        }

        let req = ret
            .body(())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Some(req))
    }
}

fn respond(req: Request<()>) -> impl Future<Item = Response<String>, Error = io::Error> + Send {
    let f = future::lazy(move || {
        let mut response = Response::builder();
        let body = match req.uri().path() {
            "/plaintext" => {
                response.header("Content-Type", "text/plain");
                "Hello, World!".to_string()
            },
            _ => {
                response.status(StatusCode::NOT_FOUND);
                String::new()
            }
        };
        let response = response
            .body(body)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        Ok(response)
    });

    Box::new(f)
}


struct HttpServer {
    listener: TcpListener,
}

impl Future for HttpServer {
    type Item = ();
    type Error = std::io::Error;
    
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.listener.poll_accept() {
                Ok(Async::Ready(t)) => {
                    tokio::spawn(process(t.0));
                },
                Ok(Async::NotReady) => {
                    return Ok(Async::NotReady);
                },
                Err(e) => {
                    return Err(From::from(e));
                }
            }
        }
    }
}