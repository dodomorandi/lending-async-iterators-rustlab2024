use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{self, Poll},
};

use pin_project::pin_project;

pub trait LendingFuture {
    type Output<'a>
    where
        Self: 'a;

    fn poll<'a>(self: Pin<&'a mut Self>, cx: &mut task::Context) -> Poll<Self::Output<'a>>;

    fn detach<F, O>(self, f: F) -> Detach<Self, F, O>
    where
        Self: Sized,
        for<'b> F: FnMut(Self::Output<'b>) -> O,
    {
        Detach {
            lending_fut: self,
            f: Some(f),
            _marker: PhantomData,
        }
    }
}

#[derive(Debug)]
#[pin_project]
pub struct Detach<T, F, O> {
    #[pin]
    lending_fut: T,
    f: Option<F>,
    _marker: PhantomData<fn() -> O>,
}

impl<T, F, O> Future for Detach<T, F, O>
where
    T: LendingFuture,
    for<'a> F: FnOnce(T::Output<'a>) -> O,
{
    type Output = O;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let value = task::ready!(this.lending_fut.poll(cx));
        let f = this
            .f
            .take()
            .expect("a Detach future can only be awaited once");
        Poll::Ready(f(value))
    }
}
