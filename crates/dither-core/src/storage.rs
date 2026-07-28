use std::{
    fs::File,
    marker::PhantomData,
    mem::size_of,
    ops::{Deref, DerefMut},
};

use bytemuck::{Pod, try_cast_slice, try_cast_slice_mut};
use memmap2::{MmapMut, MmapOptions};
use tempfile::tempfile;

pub(crate) struct Scratch<T: Pod> {
    _file: File,
    map: MmapMut,
    len: usize,
    value: PhantomData<T>,
}

impl<T: Pod> Scratch<T> {
    pub(crate) fn new(len: usize) -> std::io::Result<Self> {
        let byte_len = len
            .checked_mul(size_of::<T>())
            .ok_or_else(|| std::io::Error::other("render scratch size overflow"))?;
        let file = tempfile()?;
        file.set_len(
            byte_len
                .try_into()
                .map_err(|_| std::io::Error::other("render scratch is too large"))?,
        )?;
        let map = unsafe { MmapOptions::new().len(byte_len).map_mut(&file)? };
        Ok(Self {
            _file: file,
            map,
            len,
            value: PhantomData,
        })
    }

    #[cfg(test)]
    pub(crate) fn byte_len(&self) -> usize {
        self.map.len()
    }
}

impl<T: Pod> Deref for Scratch<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        let values = try_cast_slice(&self.map).expect("memory maps have aligned element storage");
        &values[..self.len]
    }
}

impl<T: Pod> DerefMut for Scratch<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let values =
            try_cast_slice_mut(&mut self.map).expect("memory maps have aligned element storage");
        &mut values[..self.len]
    }
}
