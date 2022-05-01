use bytes::Bytes;
use futures_util::{
    ready,
    stream::{self, TryStream},
};
use http::HeaderMap;
use http_body::Body;
use pin_project_lite::pin_project;
use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};
use sync_wrapper::SyncWrapper;

type BoxError = Box<dyn std::error::Error>;

pin_project! {
    pub struct StreamBody<S> {
        #[pin]
        stream: SyncWrapper<S>,
    }
}

unsafe impl<S> Send for StreamBody<S> {}

impl<S> From<S> for StreamBody<S>
where
    S: TryStream + 'static,
    S::Ok: Into<Bytes>,
    S::Error: Into<BoxError>,
{
    fn from(stream: S) -> Self {
        Self::new(stream)
    }
}

impl<S> StreamBody<S> {
    pub(crate) fn new(stream: S) -> Self
    where
        S: TryStream + 'static,
        S::Ok: Into<Bytes>,
        S::Error: Into<BoxError>,
    {
        Self {
            stream: SyncWrapper::new(stream),
        }
    }
}

impl Default for StreamBody<futures_util::stream::Empty<Result<Bytes, BoxError>>> {
    fn default() -> Self {
        Self::new(stream::empty())
    }
}

impl<S> fmt::Debug for StreamBody<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StreamBody").finish()
    }
}

impl<S> Body for StreamBody<S>
where
    S: TryStream,
    S::Ok: Into<Bytes>,
    S::Error: Into<BoxError>,
{
    type Data = Bytes;
    type Error = BoxError;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        let stream = self.project().stream.get_pin_mut();
        match ready!(stream.try_poll_next(cx)) {
            Some(Ok(chunk)) => Poll::Ready(Some(Ok(chunk.into()))),
            Some(Err(err)) => Poll::Ready(Some(Err(err.into()))),
            None => Poll::Ready(None),
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        Poll::Ready(Ok(None))
    }
}
