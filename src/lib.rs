#![feature(type_alias_impl_trait)]

pub mod embassy_net;
pub mod lending_future;
pub mod lending_iterator;
pub mod unsafe_pinned;

use std::{
    future::{self, poll_fn, Future},
    ops::Not,
    pin::{pin, Pin},
    task::{self, Poll},
};

use lending_future::LendingFuture;
use pin_project::pin_project;

pub trait LendingAsyncIterator {
    type Item<'a>
    where
        Self: 'a;

    fn poll_next<'a>(
        self: Pin<&'a mut Self>,
        cx: &mut task::Context,
    ) -> Poll<Option<Self::Item<'a>>>;

    #[inline]
    fn poll_progress(self: Pin<&mut Self>, _cx: &mut task::Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }

    #[inline]
    fn poll_next_unpin<'a>(&'a mut self, cx: &mut task::Context) -> Poll<Option<Self::Item<'a>>>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_next(cx)
    }

    #[must_use]
    fn next(&mut self) -> Next<'_, Self> {
        Next(self)
    }

    #[must_use]
    fn next_pinned<'a>(self: Pin<&'a mut Self>) -> NextPinned<'a, Self> {
        NextPinned(self)
    }

    fn filter<P>(self, predicate: P) -> Filter<Self, P>
    where
        Self: Sized,
        for<'a> P: FnMut(&Self::Item<'a>) -> bool,
    {
        Filter {
            async_iter: self,
            predicate,
        }
    }

    // fn for_each<F>(self, f: F) -> ForEach<Self, F>
    // where
    //     Self: Sized,
    //     for<'a> F: FnMut(Self::Item<'a>),
    // {
    //     ForEach {
    //         async_iter: self,
    //         f,
    //     }
    // }

    fn for_each<F>(self, mut f: F) -> impl Future<Output = ()>
    where
        Self: Sized,
        for<'a> F: FnMut(Self::Item<'a>),
    {
        async move {
            let mut this = pin!(self);
            poll_fn(|cx| {
                while let Some(element) = task::ready!(this.as_mut().poll_next(cx)) {
                    f(element);
                }
                Poll::Ready(())
            })
            .await;
        }
    }

    fn fold<Acc, FoldFn>(self, init: Acc, fold: FoldFn) -> Fold<Self, Acc, FoldFn>
    where
        Self: Sized,
        for<'a> FoldFn: FnMut(Acc, Self::Item<'a>) -> Acc,
    {
        Fold {
            async_iter: self,
            acc: Some(init),
            fold,
        }
    }

    fn all<F>(self, mut f: F) -> impl Future<Output = bool>
    where
        Self: Sized,
        for<'a> F: FnMut(Self::Item<'a>) -> bool,
    {
        async move {
            let mut this = pin!(self);
            future::poll_fn(move |cx| {
                while let Some(item) = task::ready!(this.as_mut().poll_next(cx)) {
                    if f(item).not() {
                        return Poll::Ready(false);
                    }
                }

                Poll::Ready(true)
            })
            .await
        }
    }

    fn any<F>(self, mut f: F) -> impl Future<Output = bool>
    where
        Self: Sized,
        for<'a> F: FnMut(Self::Item<'a>) -> bool,
    {
        async move {
            let mut this = pin!(self);
            future::poll_fn(move |cx| {
                while let Some(item) = task::ready!(this.as_mut().poll_next(cx)) {
                    if f(item) {
                        return Poll::Ready(true);
                    }
                }

                Poll::Ready(false)
            })
            .await
        }
    }
}

#[derive(Debug)]
pub struct Next<'a, T: ?Sized>(&'a mut T);

impl<T> LendingFuture for Next<'_, T>
where
    T: ?Sized + LendingAsyncIterator + Unpin,
{
    type Output<'i>
        = Option<T::Item<'i>>
    where
        Self: 'i;

    #[inline]
    fn poll<'b>(self: Pin<&'b mut Self>, cx: &mut task::Context) -> Poll<Self::Output<'b>> {
        let inner = Pin::into_inner(self);
        Pin::new(&mut *inner.0).poll_next(cx)
    }
}

#[derive(Debug)]
#[pin_project]
pub struct NextPinned<'a, T: ?Sized>(Pin<&'a mut T>);

impl<T> LendingFuture for NextPinned<'_, T>
where
    T: ?Sized + LendingAsyncIterator,
{
    type Output<'i>
        = Option<T::Item<'i>>
    where
        Self: 'i;

    #[inline]
    fn poll<'b>(self: Pin<&'b mut Self>, cx: &mut task::Context) -> Poll<Self::Output<'b>> {
        let this = self.project();
        this.0.as_mut().poll_next(cx)
    }
}

#[derive(Debug)]
#[pin_project]
#[must_use]
pub struct Filter<I, P> {
    #[pin]
    async_iter: I,
    predicate: P,
}

impl<I, P> LendingAsyncIterator for Filter<I, P>
where
    I: LendingAsyncIterator,
    for<'a> P: FnMut(&I::Item<'a>) -> bool,
{
    type Item<'a>
        = I::Item<'a>
    where
        Self: 'a;

    fn poll_next<'a>(
        self: Pin<&'a mut Self>,
        cx: &mut task::Context,
    ) -> Poll<Option<Self::Item<'a>>> {
        // let mut this = self.project();
        //
        // while let Some(element) = task::ready!(this.async_iter.as_mut().poll_next(cx)) {
        //     if (this.predicate)(&element) {
        //         return Poll::Ready(Some(element));
        //     }
        // }

        let this = self.project();
        let predicate = this.predicate;
        // SAFETY: we only use the reference once re-pinned
        let async_iter = unsafe { this.async_iter.get_unchecked_mut() };

        // SAFETY: it was pinned before, we pin it again
        while let Some(element) =
            task::ready!(unsafe { Pin::new_unchecked(&mut *async_iter) }.poll_next(cx))
        {
            if (predicate)(&element) {
                return Poll::Ready(Some(element));
            }
        }

        Poll::Ready(None)
    }
}

#[pin_project]
#[must_use]
pub struct ForEach<I, F> {
    #[pin]
    async_iter: I,
    f: F,
}

impl<I, F> Future for ForEach<I, F>
where
    I: LendingAsyncIterator,
    for<'a> F: FnMut(I::Item<'a>),
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        while let Some(element) = task::ready!(this.async_iter.as_mut().poll_next(cx)) {
            (this.f)(element);
        }

        Poll::Ready(())
    }
}

#[pin_project]
#[must_use]
pub struct Fold<I, Acc, FoldFn> {
    #[pin]
    async_iter: I,
    acc: Option<Acc>,
    fold: FoldFn,
}

impl<I, Acc, FoldFn> Future for Fold<I, Acc, FoldFn>
where
    I: LendingAsyncIterator,
    for<'a> FoldFn: FnMut(Acc, I::Item<'a>) -> Acc,
{
    type Output = Acc;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        while let Some(element) = task::ready!(this.async_iter.as_mut().poll_next(cx)) {
            *this.acc = Some((this.fold)(
                this.acc
                    .take()
                    .expect("Fold future can only be resolved once"),
                element,
            ));
        }

        Poll::Ready(
            this.acc
                .take()
                .expect("Fold future can only be resolved once"),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        marker::PhantomPinned,
        ops::{Not, Range},
        pin::{pin, Pin},
        task::{self, Poll},
        time::Duration,
    };

    use rand::{thread_rng, Rng};
    use tokio::time::{sleep, Sleep};

    use crate::lending_future::LendingFuture;

    use super::LendingAsyncIterator;

    struct DemoLendingAsyncIterator<I: Iterator> {
        iter: I,
        max_sleeps: u8,
        sleep_duration_range: Range<Duration>,
        sleep_fut: Option<Pin<Box<Sleep>>>,
        sleep_occurrence: u8,
        next_item: Option<I::Item>,
    }

    impl<I: Iterator> DemoLendingAsyncIterator<I> {
        fn new(iter: I, max_sleeps: u8, sleep_duration_range: Range<Duration>) -> Self {
            Self {
                iter,
                max_sleeps,
                sleep_duration_range,
                sleep_fut: None,
                sleep_occurrence: 0,
                next_item: None,
            }
        }
    }

    impl<I> LendingAsyncIterator for DemoLendingAsyncIterator<I>
    where
        I: Iterator + Unpin,
        I::Item: Unpin,
    {
        type Item<'a>
            = &'a mut I::Item
        where
            Self: 'a;

        fn poll_next<'a>(
            self: Pin<&'a mut Self>,
            cx: &mut task::Context,
        ) -> Poll<Option<Self::Item<'a>>> {
            let this = self.get_mut();

            if let Some(sleep_fut) = &mut this.sleep_fut {
                task::ready!(sleep_fut.as_mut().poll(cx));
                this.sleep_fut = None;
            }

            let mut rng = thread_rng();
            while this.sleep_occurrence < this.max_sleeps {
                let will_sleep = rng.gen::<bool>();
                if will_sleep {
                    this.sleep_occurrence += 1;
                    let duration = rng.gen_range(this.sleep_duration_range.clone());
                    this.sleep_fut = Some(Box::pin(sleep(duration)));

                    let Some(sleep_fut) = &mut this.sleep_fut else {
                        unreachable!();
                    };
                    task::ready!(sleep_fut.as_mut().poll(cx));
                    this.sleep_fut = None;
                }
            }

            this.sleep_occurrence = 0;
            this.next_item = this.iter.next();
            Poll::Ready(this.next_item.as_mut())
        }
    }

    #[tokio::test]
    async fn next() {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        struct A(i32, PhantomPinned);

        let mut nums = [1, 2, 3].map(|x| A(x, PhantomPinned));
        let mut demo = pin!(DemoLendingAsyncIterator::new(
            nums.iter_mut(),
            2,
            Duration::from_millis(0)..Duration::from_millis(50),
        ));

        demo.next()
            .detach(|value| {
                assert_eq!(value.unwrap().0, 1);
            })
            .await;
        demo.next()
            .detach(|value| {
                assert_eq!(value.unwrap().0, 2);
            })
            .await;
        demo.next()
            .detach(|value| {
                assert_eq!(value.unwrap().0, 3);
            })
            .await;
        demo.next()
            .detach(|value| {
                assert!(value.is_none());
            })
            .await;
    }

    #[tokio::test]
    async fn filter_for_each() {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        struct A(i32);

        let mut nums = [1, 2, 3, 4, 5, 6, 7, 8].map(A);
        DemoLendingAsyncIterator::new(
            nums.iter_mut(),
            4,
            Duration::from_millis(0)..Duration::from_millis(50),
        )
        .filter(|value| value.0 % 2 != 0)
        .for_each(|val| {
            val.0 *= 4;
        })
        .await;

        assert_eq!(nums, [4, 2, 12, 4, 20, 6, 28, 8].map(A));
    }

    #[tokio::test]
    async fn all() {
        let mut nums = [1, 2, 3, 4, 5, 6, 7, 8];
        let result = DemoLendingAsyncIterator::new(
            nums.iter_mut(),
            4,
            Duration::from_millis(0)..Duration::from_millis(50),
        )
        .all(|&mut &mut n| n < 10)
        .await;

        assert!(result);

        let result = DemoLendingAsyncIterator::new(
            nums.iter_mut(),
            4,
            Duration::from_millis(0)..Duration::from_millis(50),
        )
        .all(|&mut &mut n| n < 8)
        .await;

        assert!(result.not());
    }

    #[tokio::test]
    async fn any() {
        let mut nums = [1, 2, 3, 4, 5, 6, 7, 8];
        let result = DemoLendingAsyncIterator::new(
            nums.iter_mut(),
            4,
            Duration::from_millis(0)..Duration::from_millis(50),
        )
        .any(|&mut &mut n| n > 7)
        .await;

        assert!(result);

        let result = DemoLendingAsyncIterator::new(
            nums.iter_mut(),
            4,
            Duration::from_millis(0)..Duration::from_millis(50),
        )
        .any(|&mut &mut n| n > 8)
        .await;

        assert!(result.not());
    }
}
