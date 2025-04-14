// Copyright (C) 2025  Braiins Systems s.r.o.

#[derive(Debug)]
pub struct UsizeMetadata {
    pub default: usize,
    pub min: usize,
    pub max: usize,
}

impl UsizeMetadata {
    #[must_use]
    pub fn new(default: usize, min: usize, max: usize) -> Self {
        Self { default, min, max }
    }
}

#[derive(Debug)]
pub struct ResolutionMetadata {
    pub width: u32,
    pub height: u32,
}

impl ResolutionMetadata {
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug)]
pub struct DisplayMetadata {
    pub brightness: UsizeMetadata,
    pub resolution: ResolutionMetadata,
}

impl DisplayMetadata {
    #[must_use]
    pub fn new(brightness: UsizeMetadata, resolution: ResolutionMetadata) -> Self {
        Self {
            brightness,
            resolution,
        }
    }
}
