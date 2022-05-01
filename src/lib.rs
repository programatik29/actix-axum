use self::{service::MakeServiceRef, stream_body::StreamBody};
use actix_http::{HttpService, Payload};
use actix_server::ServerBuilder;
use actix_service::ServiceFactory;
use bytes::{Buf, Bytes};
use futures_util::future::{poll_fn, LocalBoxFuture};
use parking_lot::Mutex;
use pin_project_lite::pin_project;
use std::{
    convert::Infallible,
    fmt::{self, Debug},
    future::Future,
    mem,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

mod service;
mod stream_body;

pub struct Server<M> {
    make_service: M,
    builder: ServerBuilder,
}

impl<M: Send + Clone + 'static> Server<M> {
    pub fn new(make_service: M) -> Self {
        Self {
            make_service,
            builder: ServerBuilder::new(),
        }
    }

    pub fn listen<B>(mut self, lst: std::net::TcpListener) -> std::io::Result<Self>
    where
        M: MakeServiceRef<
            SocketAddr,
            http::Request<StreamBody<Payload>>,
            Response = http::Response<B>,
            Error = Infallible,
        >,
        M::MakeError: Debug,
        B: http_body::Body + 'static,
        B::Error: Into<Box<dyn std::error::Error>>,
    {
        let make_service = self.make_service.clone();
        let addr = lst.local_addr().unwrap();

        self.builder = self
            .builder
            .listen(format!("actix-axum-{}", addr), lst, move || {
                let make_service = make_service.clone();

                HttpService::new(MakeServiceToFactory::new(make_service)).tcp()
            })?;

        Ok(self)
    }

    pub fn run(self) -> actix_server::Server {
        self.builder.run()
    }
}

impl<M> Debug for Server<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server").finish()
    }
}

struct MakeServiceToFactory<M> {
    make_service: M,
}

impl<M> MakeServiceToFactory<M> {
    fn new(make_service: M) -> Self {
        Self { make_service }
    }
}

impl<M, B> ServiceFactory<actix_http::Request> for MakeServiceToFactory<M>
where
    M: MakeServiceRef<
            SocketAddr,
            http::Request<StreamBody<Payload>>,
            Response = http::Response<B>,
            Error = Infallible,
        > + Clone
        + 'static,
{
    type Response = actix_http::Response<HttpToActixBody<B>>;
    type Error = Infallible;
    type Config = ();
    type Service = TowerToActixService<M::Service>;
    type InitError = M::MakeError;
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        let mut make_service = self.make_service.clone();

        Box::pin(async move {
            // Just pass a dummy for now.
            let peer_addr = SocketAddr::from(([1, 1, 1, 1], 1234));

            poll_fn(|cx| make_service.poll_ready(cx)).await?;
            let service = make_service.make_service(&peer_addr).await?;

            Ok(TowerToActixService::new(service))
        })
    }
}

struct TowerToActixService<S> {
    inner: Mutex<S>,
}

impl<S> TowerToActixService<S> {
    fn new(inner: S) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

impl<S, B> actix_service::Service<actix_http::Request> for TowerToActixService<S>
where
    S: tower_service::Service<
        http::Request<StreamBody<Payload>>,
        Response = http::Response<B>,
        Error = Infallible,
    >,
{
    type Response = actix_http::Response<HttpToActixBody<B>>;
    type Error = Infallible;
    type Future = HttpToActixResponseFuture<S::Future>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.lock().poll_ready(ctx)
    }

    fn call(&self, req: actix_http::Request) -> Self::Future {
        let req = actix_to_http_request(req);
        let resp_future = self.inner.lock().call(req);

        HttpToActixResponseFuture::new(resp_future)
    }
}

pin_project! {
    struct HttpToActixResponseFuture<F> {
        #[pin]
        inner: F,
    }
}

impl<F> HttpToActixResponseFuture<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}

impl<F, B> Future for HttpToActixResponseFuture<F>
where
    F: Future<Output = Result<http::Response<B>, Infallible>>,
{
    type Output = Result<actix_http::Response<HttpToActixBody<B>>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project().inner.poll(cx) {
            Poll::Ready(http_resp) => Poll::Ready(http_resp.map(http_to_actix_response)),
            Poll::Pending => Poll::Pending,
        }
    }
}

pin_project! {
    struct HttpToActixBody<B> {
        #[pin]
        inner: B,
    }
}

impl<B> HttpToActixBody<B> {
    fn new(inner: B) -> Self {
        Self { inner }
    }
}

impl<B> actix_http::body::MessageBody for HttpToActixBody<B>
where
    B: http_body::Body,
    B::Error: Into<Box<dyn std::error::Error>>,
{
    type Error = B::Error;

    fn size(&self) -> actix_http::body::BodySize {
        if let Some(size) = self.inner.size_hint().exact() {
            if size != 0 {
                actix_http::body::BodySize::Sized(size)
            } else {
                actix_http::body::BodySize::Sized(0)
            }
        } else {
            actix_http::body::BodySize::Stream
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        http_body::Body::poll_data(self.project().inner, cx).map(|option| {
            option.map(|result| result.map(|mut buf| buf.copy_to_bytes(buf.remaining())))
        })
    }
}

fn actix_to_http_request(mut req: actix_http::Request) -> http::Request<StreamBody<Payload>> {
    let method = req.method().clone();
    let uri = mem::take(req.uri_mut());
    let version = req.version();
    let payload = req.take_payload();

    let mut http_req = http::Request::new(StreamBody::new(payload));
    *http_req.method_mut() = method;
    *http_req.uri_mut() = uri;
    *http_req.version_mut() = version;

    let mut previous = None;
    for (name, value) in req.headers_mut().drain() {
        match name {
            Some(name) => {
                http_req.headers_mut().insert(&name, value);
                previous = Some(name);
            }
            None => {
                let name = previous
                    .clone()
                    .expect("first header name in HeaderMap is none");

                http_req.headers_mut().append(name, value);
            }
        }
    }

    http_req
}

fn http_to_actix_response<B>(resp: http::Response<B>) -> actix_http::Response<HttpToActixBody<B>> {
    let (parts, body) = resp.into_parts();

    let mut actix_resp = actix_http::Response::with_body(parts.status, HttpToActixBody::new(body));
    *actix_resp.headers_mut() = actix_http::header::HeaderMap::from(parts.headers);

    actix_resp
}
