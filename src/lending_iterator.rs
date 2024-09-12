pub trait LendingIterator {
    type Item<'a>
    where
        Self: 'a;

    fn next(&mut self) -> Option<Self::Item<'_>>;
}

pub trait Ext: LendingIterator {
    fn filter<P>(self, predicate: P) -> Filter<Self, P>
    where
        Self: Sized,
        for<'a> P: FnMut(&Self::Item<'a>) -> bool;
}

impl<I> Ext for I
where
    I: LendingIterator,
{
    fn filter<P>(self, predicate: P) -> Filter<Self, P>
    where
        Self: Sized,
        for<'a> P: FnMut(&Self::Item<'a>) -> bool,
    {
        Filter {
            iter: self,
            predicate,
        }
    }
}

#[derive(Debug)]
pub struct Filter<I, P> {
    iter: I,
    predicate: P,
}

impl<I, P> LendingIterator for Filter<I, P>
where
    I: LendingIterator,
    for<'a> P: FnMut(&I::Item<'a>) -> bool,
{
    type Item<'a> = I::Item<'a>
    where
        Self: 'a;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        while let Some(element) = self.iter.next() {
            if (self.predicate)(&element) {
                return Some(element);
            }
        }

        None
    }
}
