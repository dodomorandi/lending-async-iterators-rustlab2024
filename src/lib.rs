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

    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }

    fn count(self) -> impl Future<Output = usize>
    where
        Self: Sized,
    {
        async {
            let mut this = pin!(self);
            poll_fn(move |cx| {
                let mut count = 0;
                while task::ready!(this.as_mut().poll_next(cx)).is_some() {
                    count += 1;
                }

                Poll::Ready(count)
            })
            .await
        }
    }

    fn cycle(self) -> Cycle<Self>
    where
        Self: Sized + Clone,
    {
        Cycle {
            original: self.clone(),
            current: self,
            read_item: false,
        }
    }

    fn enumerate(self) -> Enumerate<Self>
    where
        Self: Sized,
    {
        Enumerate {
            iter: self,
            index: 0,
        }
    }

    fn eq<I>(self, other: I) -> impl Future<Output = bool>
    where
        I: IntoLendingAsyncIterator,
        for<'a> Self::Item<'a>: PartialEq<I::Item<'a>>,
        Self: Sized,
    {
        self.eq_by(other, |a, b| a == b)
    }

    fn eq_by<I, F>(self, other: I, mut eq: F) -> impl Future<Output = bool>
    where
        I: IntoLendingAsyncIterator,
        for<'a> Self::Item<'a>: PartialEq<I::Item<'a>>,
        Self: Sized,
        for<'a> F: FnMut(Self::Item<'a>, I::Item<'a>) -> bool,
    {
        enum State<A, B> {
            Polling,
            AReady(A),
            BReady(B),
        }

        async move {
            let mut this = pin!(self);
            let mut other = pin!(other.into_lending_async_iter());
            let mut state = State::Polling;

            future::poll_fn(|cx| loop {
                match &state {
                    State::Polling => {
                        match (this.as_mut().poll_next(cx), other.as_mut().poll_next(cx)) {
                            (Poll::Pending, Poll::Pending) => break Poll::Pending,
                            (Poll::Ready(Some(a)), Poll::Ready(Some(b))) => {
                                if eq(a, b).not() {
                                    break Poll::Ready(false);
                                }
                            }
                            (Poll::Ready(None), Poll::Ready(None)) => break Poll::Ready(true),
                            (Poll::Ready(_), Poll::Ready(_)) => break Poll::Ready(false),
                            (Poll::Ready(a), Poll::Pending) => {
                                state = State::AReady(a);
                                break Poll::Pending;
                            }
                            (Poll::Pending, Poll::Ready(b)) => {
                                state = State::BReady(b);
                                break Poll::Pending;
                            }
                        }
                    }
                    State::AReady(_) => {
                        let b = task::ready!(other.as_mut().poll_next(cx));
                        let State::AReady(a) = std::mem::replace(&mut state, State::Polling) else {
                            unreachable!()
                        };
                        if matches!((a, b), (Some(a), Some(b)) if a == b).not() {
                            break Poll::Ready(false);
                        }
                    }
                    State::BReady(_) => {
                        let a = task::ready!(this.as_mut().poll_next(cx));
                        let State::BReady(b) = std::mem::replace(&mut state, State::Polling) else {
                            unreachable!()
                        };
                        if matches!((a, b), (Some(a), Some(b)) if a == b).not() {
                            break Poll::Ready(false);
                        }
                    }
                }
            })
            .await
        }
    }
}

impl<I> LendingAsyncIterator for &mut I
where
    I: LendingAsyncIterator,
{
    type Item<'a>
        = I::Item<'a>
    where
        Self: 'a;

    fn poll_next<'a>(
        self: Pin<&'a mut Self>,
        cx: &mut task::Context,
    ) -> Poll<Option<Self::Item<'a>>> {
        // SAFETY: see `Pin::as_deref_mut`
        let inner = unsafe { self.get_unchecked_mut() };

        // SAFETY: it was already pinned before
        let repinned = unsafe { Pin::new_unchecked(&mut **inner) };
        repinned.poll_next(cx)
    }
}

pub trait IntoLendingAsyncIterator {
    type Item<'a>;
    type IntoLendingAsyncIter: for<'a> LendingAsyncIterator<Item<'a> = Self::Item<'a>>;

    fn into_lending_async_iter(self) -> Self::IntoLendingAsyncIter;
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

#[derive(Debug, Clone)]
#[pin_project]
pub struct Cycle<I> {
    original: I,
    #[pin]
    current: I,
    read_item: bool,
}

impl<I> LendingAsyncIterator for Cycle<I>
where
    I: LendingAsyncIterator + Clone,
{
    type Item<'a>
        = I::Item<'a>
    where
        Self: 'a;

    fn poll_next<'a>(
        self: Pin<&'a mut Self>,
        cx: &mut task::Context,
    ) -> Poll<Option<Self::Item<'a>>> {
        let this = self.project();
        // SAFETY: not going to overwrite this, only reusing this after pinning it
        let current = unsafe { Pin::get_unchecked_mut(this.current) };
        // SAFETY: already pinned, projecting pin after reborrow
        match task::ready!(unsafe { Pin::new_unchecked(&mut *current) }.poll_next(cx)) {
            None if *this.read_item => {
                // SAFETY: already pinned, in this branch the reference is already dropped
                // let mut current = unsafe { Pin::new_unchecked(&mut *current) };
                // current.set(this.original.clone());
                // current.poll_next(cx)
                todo!()
            }
            None => Poll::Ready(None),
            item => {
                *this.read_item = true;
                Poll::Ready(item)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[pin_project]
pub struct Enumerate<I> {
    #[pin]
    iter: I,
    index: usize,
}

impl<I> LendingAsyncIterator for Enumerate<I>
where
    I: LendingAsyncIterator,
{
    type Item<'a>
        = (usize, I::Item<'a>)
    where
        Self: 'a;

    fn poll_next<'a>(
        self: Pin<&'a mut Self>,
        cx: &mut task::Context,
    ) -> Poll<Option<Self::Item<'a>>> {
        let this = self.project();
        this.iter.poll_next(cx).map(|opt_item| {
            opt_item.map(|item| {
                let index = *this.index;
                *this.index += 1;

                (index, item)
            })
        })
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
