use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Decay {
    #[default]
    Studio,
    Snappy,
    Smooth,
}

impl Decay {
    pub(crate) fn from_id(id: &str) -> Self {
        match id {
            "snappy" => Self::Snappy,
            "smooth" => Self::Smooth,
            _ => Self::Studio,
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Studio => "studio",
            Self::Snappy => "snappy",
            Self::Smooth => "smooth",
        }
    }

    pub(crate) fn hold_frames(self) -> u8 {
        match self {
            Self::Studio => 10,
            Self::Snappy => 4,
            Self::Smooth => 18,
        }
    }

    pub(crate) fn factor(self) -> f32 {
        match self {
            Self::Studio => 0.92,
            Self::Snappy => 0.80,
            Self::Smooth => 0.96,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Options {
    pub(crate) spin: bool,
    pub(crate) columns: usize,
    pub(crate) decay: Decay,
}

thread_local! {
    static OPTIONS: Cell<Options> = const { Cell::new(Options {
        spin: true,
        columns: 32,
        decay: Decay::Studio,
    }) };
    static OBSERVERS: RefCell<Vec<Weak<dyn Fn()>>> = RefCell::new(Vec::new());
}

pub(crate) fn current() -> Options {
    OPTIONS.get()
}

pub(crate) fn configure(spin: bool, columns: u32, decay: &str) {
    let options = Options {
        spin,
        columns: if columns == 16 { 16 } else { 32 },
        decay: Decay::from_id(decay),
    };
    if OPTIONS.replace(options) == options {
        return;
    }
    let callbacks = OBSERVERS.with_borrow_mut(|observers| {
        let callbacks: Vec<_> = observers.iter().filter_map(Weak::upgrade).collect();
        observers.retain(|observer| observer.strong_count() > 0);
        callbacks
    });
    for callback in callbacks {
        callback();
    }
}

pub(crate) fn observe(observer: &Rc<dyn Fn()>) {
    OBSERVERS.with_borrow_mut(|observers| {
        observers.retain(|registered| registered.strong_count() > 0);
        if !observers
            .iter()
            .any(|registered| registered.ptr_eq(&Rc::downgrade(observer)))
        {
            observers.push(Rc::downgrade(observer));
        }
    });
}

#[cfg(test)]
mod tests;
