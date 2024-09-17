use std::{
    future::Future,
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
    task::{self, Poll},
};

use pin_project::pin_project;

use crate::unsafe_pinned::UnsafePinned;

pub trait LendingFuture {
    type Output<'a>
    where
        Self: 'a;

    fn poll<'a>(self: Pin<&'a mut Self>, cx: &mut task::Context) -> Poll<Self::Output<'a>>;

    fn then<F, Fut, O>(self, f: F) -> Then<Self, F, Fut, O>
    where
        Self: Sized,
        for<'b> F: AsyncFnOnce<(Self::Output<'b>,), CallOnceFuture = Fut, Output = O>,
    {
        Then {
            inner: UnsafePinned::new(ThenInner {
                lending_fut: self,
                f: Some(f),
            }),
            fut: None,
            _marker: PhantomData,
        }
    }
}

#[derive(Debug)]
#[pin_project]
pub struct Then<T, F, Fut, O> {
    #[pin]
    inner: UnsafePinned<ThenInner<T, F>>,
    #[pin]
    fut: Option<Fut>,
    _marker: PhantomData<fn() -> O>,
}

#[derive(Debug)]
struct ThenInner<T, F> {
    lending_fut: T,
    f: Option<F>,
}

impl<T, F, Fut, O> Future for Then<T, F, Fut, O>
where
    T: LendingFuture,
    Fut: Future<Output = O>,
    for<'a> F: AsyncFnOnce<(<T as LendingFuture>::Output<'a>,), CallOnceFuture = Fut, Output = O>,
{
    type Output = O;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        if let Some(fut) = this.fut.as_mut().as_pin_mut() {
            fut.poll(cx)
        } else {
            let inner = this.inner.get_mut_pinned();
            // SAFETY: This is not going to alias with the future, because it is not actually alive.
            let inner = unsafe { &mut *inner };

            // SAFETY: we never move the lending future
            let lending_fut = unsafe { Pin::new_unchecked(&mut inner.lending_fut) };
            let output = task::ready!(lending_fut.poll(cx));
            let f = inner
                .f
                .take()
                .expect("it should not be possible to poll the lending future more than once");

            this.fut.set(Some(f(output)));
            let Some(fut) = this.fut.as_pin_mut() else {
                unreachable!()
            };
            fut.poll(cx)
        }
    }
}
