use std::simd::f32x8;
use std::simd::prelude::SimdFloat;

use half::f16;

use super::buf::VecDotF32;

#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct BlockQ8_0 {
    d: f16,       // delta
    qs: [i8; 32], // quants
}

impl BlockQ8_0 {
    pub const BLOCK_ELEMS: usize = 32;

    pub fn from_bytes(data: &[u8]) -> &[BlockQ8_0] {
        let size = std::mem::size_of::<BlockQ8_0>();
        assert!(
            data.len() % size == 0,
            "data length must be a multiple of QuantBlockQ8_0 size"
        );
        unsafe { std::slice::from_raw_parts(data.as_ptr() as *const BlockQ8_0, data.len() / size) }
    }

    pub fn quantize(data: &[f32]) -> Vec<BlockQ8_0> {
        let mut bs: Vec<BlockQ8_0> = vec![];
        let chunks = data.chunks(32);
        for chunk in chunks {
            let mut max = f32::MIN;
            for i in 0..32 {
                if chunk[i] > max {
                    max = chunk[i];
                }
            }
            let d = f16::from_f32(max / 127.0);
            let mut qs = [0_i8; 32];
            for i in 0..32 {
                qs[i] = (chunk[i] / d.to_f32()).round() as i8;
            }
            bs.push(BlockQ8_0 { d, qs })
        }
        bs
    }

    pub fn dequantize(&self, buf: &mut [f32]) {
        let d = self.d.to_f32();
        for i in 0..32 {
            let q = self.qs[i];
            buf[i] = q as f32 * d;
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuantBufQ8_0<'a> {
    raw: &'a [u8],
    num_blocks: usize,
}

impl<'a> QuantBufQ8_0<'a> {
    pub fn from_bytes(buf: &'a [u8]) -> Self {
        let block_mem = std::mem::size_of::<BlockQ8_0>();
        // assert!(buf.len() % block_mem == 0);
        let num_blocks = buf.len() / block_mem;
        Self {
            raw: buf,
            num_blocks,
        }
    }

    pub fn blocks(&self) -> &[BlockQ8_0] {
        BlockQ8_0::from_bytes(self.raw)
    }

    pub fn blocks_range(&self, start: usize, end: usize) -> &[BlockQ8_0] {
        &self.blocks()[start / 32..end / 32]
    }

    pub fn len(&self) -> usize {
        self.num_blocks * 32
    }

    pub fn iter_range(
        &'a self,
        start: usize,
        end: usize,
        step: usize,
    ) -> impl Iterator<Item = f32> + 'a {
        BlockBufIterQ8_0 {
            buf: &self,
            pos: start,
            end: end,
            step: step,
            current_f32_buf: [0.0; 32],
            current_block: usize::MAX,
        }
    }
}

impl<'a> VecDotF32 for QuantBufQ8_0<'a> {
    fn vec_dot_f32(&self, offset: usize, x: &[f32]) -> f32 {
        let blocks = BlockQ8_0::from_bytes(self.raw);
        let row = &blocks[offset / 32..(offset + x.len()) / 32];
        assert!(row.len() * 32 == x.len());
        let mut sum = 0.0;
        for i in 0..row.len() {
            let block = &row[i];
            let d = block.d.to_f32();
            let mut sum_block = 0.0;
            for j in 0..4 {
                let qs = &block.qs[j * 8..(j + 1) * 8];
                let qv = f32x8::from_array([
                    qs[0] as f32,
                    qs[1] as f32,
                    qs[2] as f32,
                    qs[3] as f32,
                    qs[4] as f32,
                    qs[5] as f32,
                    qs[6] as f32,
                    qs[7] as f32,
                ]);
                let xv = f32x8::from_slice(&x[i * 32 + j * 8..i * 32 + (j + 1) * 8]);
                sum_block += (qv * xv).reduce_sum();
            }
            sum += sum_block * d;
        }
        sum
    }
}

pub fn vec_dot_q8_0_f16(w: &[BlockQ8_0], x: &[f16]) -> f32 {
    let mut sum = 0.0;
    for (xb, wb) in x.chunks(32).zip(w.iter()) {
        let mut sum_block = 0.0;
        for j in 0..4 {
            let qv = f32x8::from_array([
                wb.qs[j * 8] as f32,
                wb.qs[j * 8 + 1] as f32,
                wb.qs[j * 8 + 2] as f32,
                wb.qs[j * 8 + 3] as f32,
                wb.qs[j * 8 + 4] as f32,
                wb.qs[j * 8 + 5] as f32,
                wb.qs[j * 8 + 6] as f32,
                wb.qs[j * 8 + 7] as f32,
            ]);
            let xv = f32x8::from_array([
                xb[j * 8].to_f32(),
                xb[j * 8 + 1].to_f32(),
                xb[j * 8 + 2].to_f32(),
                xb[j * 8 + 3].to_f32(),
                xb[j * 8 + 4].to_f32(),
                xb[j * 8 + 5].to_f32(),
                xb[j * 8 + 6].to_f32(),
                xb[j * 8 + 7].to_f32(),
            ]);
            sum_block += (qv * xv).reduce_sum();
        }
        sum += sum_block * wb.d.to_f32();
    }
    sum
}

pub fn vec_dot_q8_0_q8_0(w: &[BlockQ8_0], x: &[BlockQ8_0]) -> f32 {
    let mut sum = 0.0;
    for (xb, wb) in x.iter().zip(w.iter()) {
        let mut sum_block = 0.0;
        for j in 0..4 {
            let qv = f32x8::from_array([
                wb.qs[j * 8] as f32,
                wb.qs[j * 8 + 1] as f32,
                wb.qs[j * 8 + 2] as f32,
                wb.qs[j * 8 + 3] as f32,
                wb.qs[j * 8 + 4] as f32,
                wb.qs[j * 8 + 5] as f32,
                wb.qs[j * 8 + 6] as f32,
                wb.qs[j * 8 + 7] as f32,
            ]);
            let xv = f32x8::from_array([
                xb.qs[j * 8] as f32,
                xb.qs[j * 8 + 1] as f32,
                xb.qs[j * 8 + 2] as f32,
                xb.qs[j * 8 + 3] as f32,
                xb.qs[j * 8 + 4] as f32,
                xb.qs[j * 8 + 5] as f32,
                xb.qs[j * 8 + 6] as f32,
                xb.qs[j * 8 + 7] as f32,
            ]);
            sum_block += (qv * xv).reduce_sum();
        }
        sum += sum_block * wb.d.to_f32() * xb.d.to_f32();
    }
    sum
}

pub struct BlockBufIterQ8_0<'a> {
    buf: &'a QuantBufQ8_0<'a>,
    current_f32_buf: [f32; 32],
    current_block: usize,
    pos: usize,
    end: usize,
    step: usize,
}

impl<'a> Iterator for BlockBufIterQ8_0<'a> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.end {
            return None;
        }

        let block_idx = self.pos / BlockQ8_0::BLOCK_ELEMS;
        if block_idx != self.current_block {
            let block = &self.buf.blocks()[block_idx];
            block.dequantize(&mut self.current_f32_buf);
            self.current_block = block_idx;
        }

        let val = self.current_f32_buf[self.pos % 32];
        self.pos += self.step;
        Some(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_q80_block() {
        let mut buf: [u8; 68] = [0x1; 68];
        let d = f16::from_f32(3.0).to_bits().to_le_bytes();
        buf[0] = d[0];
        buf[1] = d[1];
        buf[2] = 2;
        buf[3] = 3;
        buf[4] = 4;
        buf[2 + 31] = 7;
        buf[34] = d[0];
        buf[35] = d[1];
        buf[66] = 9;
        buf[67] = 9;

        let blocks = BlockQ8_0::from_bytes(&buf[0..34]);
        assert_eq!(blocks[0].d.to_f32(), 3.0);
        assert_eq!(blocks[0].qs, [
            2, 3, 4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 7
        ]);

        let bf = QuantBufQ8_0::from_bytes(&buf);
        assert_eq!(bf.len(), 64);
        assert_eq!(bf.iter_range(0, bf.len(), 1).collect::<Vec<_>>(), vec![
            6.0, 9.0, 12.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0,
            3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 21.0, 3.0, 3.0,
            3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0,
            3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 27.0, 27.0
        ]);
        assert_eq!(bf.iter_range(10, bf.len(), 1).collect::<Vec<_>>().len(), 54);
    }
}
