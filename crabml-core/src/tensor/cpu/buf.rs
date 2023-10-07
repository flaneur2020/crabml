use std::slice;

#[derive(Debug)]
pub enum CpuTensorBuf<'a, T: Copy> {
    Owned(Vec<T>),
    Flat(&'a [T]),
    // Quantized8,
}

impl<'a, T: Copy> CpuTensorBuf<'a, T> {
    pub fn at_unchecked(&self, pos: usize) -> T {
        match self {
            CpuTensorBuf::Owned(buf) => buf[pos],
            CpuTensorBuf::Flat(buf) => buf[pos],
        }
    }

    pub fn is_owned(&self) -> bool {
        match self {
            CpuTensorBuf::Owned(_) => true,
            _ => false,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            CpuTensorBuf::Owned(buf) => buf.len(),
            CpuTensorBuf::Flat(buf) => buf.len(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        match self {
            CpuTensorBuf::Owned(buf) => buf.iter(),
            CpuTensorBuf::Flat(buf) => buf.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        match self {
            CpuTensorBuf::Owned(buf) => buf.iter_mut(),
            CpuTensorBuf::Flat(_) => unreachable!("only owned buffers can be mutable"),
        }
    }
}

impl<'a> CpuTensorBuf<'a, f32> {
    pub fn from_raw_bytes(buf: &'a [u8]) -> Self {
        let len = buf.len();
        assert_eq!(
            len % std::mem::size_of::<f32>(),
            0,
            "Length of slice must be multiple of f32 size"
        );
        let new_len = len / std::mem::size_of::<f32>();
        let ptr = buf.as_ptr() as *const f32;
        let f32_buf = unsafe { slice::from_raw_parts(ptr, new_len) };
        f32_buf.into()
    }
}

impl<T: Copy> From<Vec<T>> for CpuTensorBuf<'_, T> {
    fn from(buf: Vec<T>) -> Self {
        Self::Owned(buf)
    }
}

impl<'a, T: Copy> From<&'a [T]> for CpuTensorBuf<'a, T> {
    fn from(buf: &'a [T]) -> Self {
        Self::Flat(buf)
    }
}