use half::f16;
use rayon::prelude::*;

use crate::backends::cpu::buf::buf_f16::quantize_f32_f16;
use crate::backends::cpu::buf::buf_f16::vec_dot_f16_f16;
use crate::backends::cpu::buf::buf_f16::vec_fma_f16_f16;
use crate::backends::cpu::buf::buf_f32::vec_dot_f32_f32_strided;
use crate::backends::cpu::buf::CpuTensorBuf;
use crate::backends::cpu::CpuTensorDeviceRef;
use crate::gguf::GGMLType;
use crate::tensor::TensorStrider;

/// A (b, m, n) @ B (b, k, n) -> C (b, m, n)
///
/// A is expected to be contiguous, B is allowed to be strided, but B should
/// be contiguous on the K dimension or N dimension.
pub fn batch_matmul<'a>(
    _device: &CpuTensorDeviceRef<'a>,
    bufa: &CpuTensorBuf<'a>,
    bufb: &CpuTensorBuf<'a>,
    bufc: &mut CpuTensorBuf<'a>,
    strider1: &TensorStrider,
    strider2: &TensorStrider,
) {
    assert!(strider1.dims() == 3);
    assert!(strider2.dims() == 3);
    assert!(strider1.is_contiguous());
    assert!(strider2.strides()[1] == 1 || strider2.strides()[2] == 1);
    assert!(bufa.dtype() == GGMLType::F32 || bufa.dtype() == GGMLType::F16);
    assert!(bufb.dtype() == GGMLType::F32 || bufb.dtype() == GGMLType::F16);

    match bufb {
        CpuTensorBuf::F32(bufb) => batch_matmul_naive_f32(
            bufa.as_f32_ref(),
            bufb,
            bufc.as_f32_mut(),
            strider1,
            &strider2,
        ),
        CpuTensorBuf::F16(bufb) => {
            let bufa = quantize_f32_f16(bufa.as_f32_ref());
            batch_matmul_simd_f16(&bufa, bufb, bufc.as_f32_mut(), strider1, &strider2)
        }
        _ => unreachable!(),
    }
}

fn batch_matmul_naive_f32(
    bufa: &[f32],     // b x m x k
    bufb: &[f32],     // b x k x n
    bufc: &mut [f32], // b x m x n
    stride1: &TensorStrider,
    stride2: &TensorStrider,
) {
    let (a_batch, b_batch) = (stride1.shape()[0], stride2.shape()[0]);
    assert!(a_batch >= b_batch);
    let (m, k, n) = (stride1.shape()[1], stride1.shape()[2], stride2.shape()[2]);
    for bi in 0..a_batch {
        for mi in 0..m {
            for ni in 0..n {
                for ki in 0..k {
                    bufc[bi * (m * n) + mi * n + ni] += bufa[bi * stride1.strides()[0]
                        + mi * stride1.strides()[1]
                        + ki * stride1.strides()[2]]
                        * bufb[(bi % b_batch) * stride2.strides()[0]
                            + ki * stride2.strides()[1]
                            + ni * stride2.strides()[2]];
                }
            }
        }
    }
}

fn batch_matmul_naive_f16(
    bufa: &[f16],     // b x m x k
    bufb: &[f16],     // b x k x n
    bufc: &mut [f32], // b x m x n
    stride1: &TensorStrider,
    stride2: &TensorStrider,
) {
    let (a_batch, b_batch) = (stride1.shape()[0], stride2.shape()[0]);
    assert!(a_batch >= b_batch);
    let (m, k, n) = (stride1.shape()[1], stride1.shape()[2], stride2.shape()[2]);
    for bi in 0..a_batch {
        for mi in 0..m {
            for ni in 0..n {
                for ki in 0..k {
                    let a_val = bufa[bi * stride1.strides()[0]
                        + mi * stride1.strides()[1]
                        + ki * stride1.strides()[2]];
                    let b_val = bufb[(bi % b_batch) * stride2.strides()[0]
                        + ki * stride2.strides()[1]
                        + ni * stride2.strides()[2]];
                    bufc[bi * (m * n) + mi * n + ni] += (a_val * b_val).to_f32();
                }
            }
        }
    }
}
fn batch_matmul_simd_f16(
    bufa: &[f16],     // b x m x k
    bufb: &[f16],     // b x k x n
    bufc: &mut [f32], // b x m x n
    stride1: &TensorStrider,
    stride2: &TensorStrider,
) {
    let (a_batch, b_batch) = (stride1.shape()[0], stride2.shape()[0]);
    assert!(a_batch >= b_batch);
    let (m, k, n) = (stride1.shape()[1], stride1.shape()[2], stride2.shape()[2]);
    let (stride_bb, stride_bk, stride_bn) = (
        stride2.strides()[0],
        stride2.strides()[1],
        stride2.strides()[2],
    );

    let mut tmpc = vec![f16::ZERO; b_batch * m * n]; // TODO: avoid allocation

    // matrix A is always row-wise contiguous, matrix B should be contiguous on the K
    // dimension or N dimension.
    // if matrix B is contiguous on the k dimension, then we can use vec_dot_f16_f16
    if stride_bk == 1 {
        tmpc.par_iter_mut().enumerate().for_each(|(i, bufcp)| {
            let ni = i % n;
            let mi = (i - ni) / n % m;
            let bi = (i - ni - mi * n) / (m * n);
            let offset_a = bi * (m * k) + mi * k;
            let offset_b = (bi % b_batch) * stride_bb + ni * stride_bn;
            *bufcp = f16::from_f32(vec_dot_f16_f16(
                bufa,
                offset_a,
                &bufb[offset_b..offset_b + k],
                0,
                k,
            ));
        });
    } else if stride_bn == 1 {
        for bi in 0..a_batch {
            for mi in 0..m {
                for ki in 0..k {
                    let offset_a = bi * (m * k) + mi * k + ki;
                    let offset_b = (bi % b_batch) * stride_bb + ki * stride_bk;
                    let offset_c = bi * (m * n) + mi * n;
                    vec_fma_f16_f16(
                        &bufb[offset_b..offset_b + n],
                        bufa[offset_a],
                        &mut tmpc[offset_c..offset_c + n],
                        0,
                        n,
                    );
                }
            }
        }
    } else {
        unreachable!()
    }

    bufc.iter_mut().zip(tmpc.iter()).for_each(|(c, tmp)| {
        *c = tmp.to_f32();
    });
}

#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
fn gemv_strided_3d_2d_f32(
    _device: &CpuTensorDeviceRef,
    abuf: &[f32],     // a_batch x M x K
    bbuf: &[f32],     // b_batch x K
    cbuf: &mut [f32], // b_batch x M
    a_batch: usize,
    _b_batch: usize,
    m: usize,
    k: usize,
    bi_stride: usize,
    mi_stride: usize,
    ki_stride: usize,
) {
    cbuf.par_iter_mut().enumerate().for_each(|(i, bufcp)| {
        let mi = i % m;
        let bi = (i - mi) / m;
        *bufcp = vec_dot_f32_f32_strided(
            abuf,
            (bi % a_batch) * bi_stride + mi * mi_stride,
            ki_stride,
            k,
            &bbuf[bi * k..(bi + 1) * k],
        );
    });
}
