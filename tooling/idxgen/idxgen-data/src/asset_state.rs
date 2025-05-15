// Copyright (C) 2024  Braiins Systems s.r.o.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(untagged)]
#[serde(from = "Option<YankedOrAvailable<T>>")]
#[serde(into = "Option<YankedOrAvailable<T>>")]
pub enum AssetState<T: Clone> {
    #[default]
    None,
    Yanked,
    Available(T),
}

impl<T: Clone> AssetState<T> {
    pub const fn as_ref(&self) -> AssetState<&T> {
        match *self {
            AssetState::None => AssetState::None,
            AssetState::Yanked => AssetState::Yanked,
            AssetState::Available(ref x) => AssetState::Available(x),
        }
    }
}

impl<'a, T: Clone> From<&'a AssetState<T>> for Option<&'a T> {
    fn from(val: &'a AssetState<T>) -> Self {
        match val {
            AssetState::None | AssetState::Yanked => None,
            AssetState::Available(asset) => Some(asset),
        }
    }
}

impl<'a, T: Clone> From<AssetState<&'a T>> for Option<&'a T> {
    fn from(val: AssetState<&'a T>) -> Self {
        match val {
            AssetState::None | AssetState::Yanked => None,
            AssetState::Available(asset) => Some(asset),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum YankedOrAvailable<T> {
    Yanked { yanked: bool },
    Available(T),
}

impl<T: Clone> From<Option<YankedOrAvailable<T>>> for AssetState<T> {
    fn from(opt: Option<YankedOrAvailable<T>>) -> Self {
        match opt {
            None => AssetState::None,
            Some(YankedOrAvailable::Yanked { .. }) => AssetState::Yanked,
            Some(YankedOrAvailable::Available(t)) => AssetState::Available(t),
        }
    }
}

impl<T: Clone> From<AssetState<T>> for Option<YankedOrAvailable<T>> {
    fn from(val: AssetState<T>) -> Self {
        match val {
            AssetState::<T>::None => None,
            AssetState::<T>::Yanked => Some(YankedOrAvailable::Yanked { yanked: true }),
            AssetState::<T>::Available(data) => Some(YankedOrAvailable::Available(data)),
        }
    }
}
