#![feature(type_alias_impl_trait)]

pub mod embassy_net;
pub mod lending_iterator;

use std::{
    future::Future,
    pin::{pin, Pin},
    task::{self, Poll},
};

use pin_project::pin_project;

pub trait LendingFuture {
    type Item<'a>
    where
        Self: 'a;

    fn poll<'a>(self: Pin<&'a mut Self>, cx: &mut task::Context) -> Poll<Self::Item<'a>>;
}

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
}

pub trait LendingAsyncIteratorExt: LendingAsyncIterator {
    fn next(&mut self) -> Next<'_, Self>
    where
        Self: Unpin;

    fn filter<P>(self, predicate: P) -> Filter<Self, P>
    where
        Self: Sized,
        for<'a> P: FnMut(&Self::Item<'a>) -> bool;

    fn for_each<F>(self, f: F) -> ForEach<Self, F>
    where
        Self: Sized,
        for<'a> F: FnMut(Self::Item<'a>);

    // fn for_each_async<F>(self, f: F) -> ForEachAsync<Self, F>
    // where
    //     Self: Sized,
    //     for<'a> F: AsyncFnMut(Self::Item<'a>);
}

impl<I> LendingAsyncIteratorExt for I
where
    I: LendingAsyncIterator,
{
    fn next(&mut self) -> Next<'_, Self>
    where
        Self: Unpin,
    {
        Next(self)
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

    fn for_each<F>(self, f: F) -> ForEach<Self, F>
    where
        Self: Sized,
        for<'a> F: FnMut(Self::Item<'a>),
    {
        ForEach {
            async_iter: self,
            f,
        }
    }

    // fn for_each_async<F>(self, f: F) -> ForEachAsync<Self, F>
    // where
    //     Self: Sized,
    //     for<'a> F: AsyncFnMut(Self::Item<'a>),
    // {
    //     ForEachAsync {
    //         async_iter: self,
    //         f,
    //         fut: None,
    //     }
    // }
}

#[derive(Debug)]
pub struct Next<'a, T: ?Sized>(&'a mut T);

impl<'a, T> LendingFuture for Next<'a, T>
where
    T: ?Sized + LendingAsyncIterator + Unpin,
{
    type Item<'i> = Option<T::Item<'i>>
    where
        Self: 'i;

    #[inline]
    fn poll<'b>(self: Pin<&'b mut Self>, cx: &mut task::Context) -> Poll<Self::Item<'b>> {
        Pin::into_inner(self).0.poll_next_unpin(cx)
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
    type Item<'a> = I::Item<'a>
    where
        Self: 'a;

    fn poll_next<'a>(
        self: Pin<&'a mut Self>,
        cx: &mut task::Context,
    ) -> Poll<Option<Self::Item<'a>>> {
        // let this = self.project();
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

pub trait Captures<T> {}
impl<T, U: ?Sized> Captures<T> for U {}

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

// #[pin_project]
// #[must_use]
// pub struct ForEachAsync<I, F, Fut> {
//     #[pin]
//     async_iter: I,
//     f: F,
//     #[pin]
//     fut: Option<Fut>,
// }

// impl<I, F, Fut> Future for ForEachAsync<I, F, Fut>
// where
//     I: LendingAsyncIterator,
//     for<'a> F: AsyncFnMut<(I::Item<'a>,), CallMutFuture<'a> = Fut>,
// {
//     type Output = ();
//
//     fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
//         todo!()
//     }
// }

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        ops::Range,
        pin::Pin,
        task::{self, Poll},
        time::Duration,
    };

    use rand::{thread_rng, Rng};
    use tokio::time::{sleep, Sleep};

    use crate::{LendingAsyncIterator, LendingAsyncIteratorExt};

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
        type Item<'a> = &'a mut I::Item
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
    async fn test() {
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
}
