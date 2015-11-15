use std::iter::{Iterator, Peekable};
use std::cmp::Eq;


// An iterator that yields the run length and the element iself
// 
// Iterator element type is `(usize, I::Item)`
pub struct RunLength<I>
    where I: Iterator
{
    iter: Peekable<I>
}

impl<I, T> Iterator for RunLength<I>
    where I: Iterator<Item=T>, T: Eq
{
    type Item = (usize, I::Item);

    fn next(&mut self) -> Option<(usize, I::Item)> {
        let current = match self.iter.next() {
            Some(current) => current,
            None => return None,
        };

        let mut length = 1;

        while self.iter.peek() == Some(&current) {
            length += 1;
            self.iter.next();
        }

        Some((length, current))
    }
}

impl<I> RunLength<I>
    where I: Iterator
{
    fn new(i: I) -> RunLength<I> {
        RunLength { iter: i.peekable() }
    }
}

pub trait RunLengthIterator: Iterator {
    fn run_length(self) -> RunLength<Self>
        where Self: Sized
    {
        RunLength::new(self)
    }
}

impl<T: ?Sized> RunLengthIterator for T where T: Iterator { }


#[test]
fn test_run_length() {
    let numbers = [
        1,
        2, 2,
        3, 3, 3,
        4, 4, 4, 4
    ];

    for (length, number) in numbers.iter().run_length() {
        assert_eq!(length, *number);
    }

    assert_eq!(None, "".chars().run_length().next());
    assert_eq!(Some((1, '1')), "1".chars().run_length().next());
}
