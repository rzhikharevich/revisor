#![allow(dead_code)]

use std::cell::Cell;
use std::iter;

pub fn skip_slice_until<T, F>(xs: &[T], pred: F) -> &[T]
where
    F: FnMut(&T) -> bool,
{
    xs.iter().position(pred).map_or(&[], |pos| &xs[pos..])
}

pub fn mutate_cell<T: Default, F: FnOnce(&mut T)>(c: &Cell<T>, f: F) {
    let mut x = c.take();
    f(&mut x);
    c.set(x);
}

pub fn zero<T: iter::Sum>() -> T {
    iter::empty().sum()
}

pub trait IsNegative {
    fn is_negative(&self) -> bool;
}

impl<T: Ord + iter::Sum> IsNegative for T {
    fn is_negative(&self) -> bool {
        self < &zero::<T>()
    }
}

pub struct Defer<F: FnMut()> {
    f: F,
}

impl<F: FnMut()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.f)();
    }
}

pub fn defer<F: FnMut()>(f: F) -> Defer<F> {
    Defer { f }
}

pub trait Args: TryFrom<getopts::Matches, Error = ()> {
    fn need_help(&self) -> bool;
}

pub fn parse_args<A: Args>(usage: &str, mut opts: getopts::Options) -> Result<A, ()> {
    opts.optflag("h", "help", "print this help text");

    let matches = opts.parse(std::env::args_os().skip(1)).map_err(|err| {
        eprint!("Error: {}\n\n{}", err, opts.usage(usage));
    })?;

    let parsed_args = A::try_from(matches)?;
    if parsed_args.need_help() {
        eprint!("{}", opts.usage(usage));
    }
    Ok(parsed_args)
}
