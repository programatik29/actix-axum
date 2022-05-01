use std::{
    future::Future,
    task::{Context, Poll},
};
use tower_service::Service;

/// Modified version of [`MakeService`] that takes a `&Target`.
///
/// This trait is sealed and cannot be implemented for types outside this crate.
///
/// [`MakeService`]: https://docs.rs/tower/0.4/tower/make/trait.MakeService.html
#[allow(missing_docs)]
pub trait MakeServiceRef<Target, Request>: make_service_ref::Sealed<(Target, Request)> {
    type Response;
    type Error;
    type Service: Service<Request, Response = Self::Response, Error = Self::Error>;
    type MakeError;
    type Future: Future<Output = Result<Self::Service, Self::MakeError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::MakeError>>;

    fn make_service(&mut self, target: &Target) -> Self::Future;
}

impl<T, S, ME, F, Target, Request> make_service_ref::Sealed<(Target, Request)> for T
where
    T: for<'a> Service<&'a Target, Response = S, Error = ME, Future = F>,
    S: Service<Request>,
    F: Future<Output = Result<S, ME>>,
{
}

impl<T, S, ME, F, Target, Request> MakeServiceRef<Target, Request> for T
where
    T: for<'a> Service<&'a Target, Response = S, Error = ME, Future = F>,
    S: Service<Request>,
    F: Future<Output = Result<S, ME>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Service = S;
    type MakeError = ME;
    type Future = F;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::MakeError>> {
        self.poll_ready(cx)
    }

    fn make_service(&mut self, target: &Target) -> Self::Future {
        self.call(target)
    }
}

mod make_service_ref {
    pub trait Sealed<T> {}
}
