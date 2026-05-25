#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    improper_ctypes,
    dead_code,
    unused_imports
)]
mod yuv_ffi {
    include!(concat!(env!("OUT_DIR"), "/yuv_ffi.rs"));
}
use yuv_ffi::*;

macro_rules! call_yuv {
    ($x:expr) => {{
        let result = unsafe { $x };
        let result_int = unsafe { std::mem::transmute::<_, i32>(result) };
        if result_int != 0 {
            return Err(format!(
                "libyuv call failed: errcode={} at {}:{}",
                result_int,
                file!(),
                line!()
            )
            .into());
        }
        result
    }};
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pixfmt {
    BGRA,
    RGBA,
    I420,
}

impl Pixfmt {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Pixfmt::BGRA | Pixfmt::RGBA => 4,
            Pixfmt::I420 => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EncodeYuvFormat {
    pub pixfmt: Pixfmt,
    pub w: usize,
    pub h: usize,
    pub stride: Vec<usize>,
    pub u: usize,
    pub v: usize,
}

pub trait TraitPixelBuffer {
    fn data(&self) -> &[u8];
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    fn stride(&self) -> Vec<usize>;
    fn pixfmt(&self) -> Pixfmt;
}

pub fn convert_to_yuv(
    captured: &dyn TraitPixelBuffer,
    dst_fmt: &EncodeYuvFormat,
    dst: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let src = captured.data();
    let src_stride = captured.stride();
    let src_width = captured.width();
    let src_height = captured.height();

    if captured.pixfmt() != Pixfmt::BGRA || dst_fmt.pixfmt != Pixfmt::I420 {
        return Err(format!(
            "unsupported conversion {:?} -> {:?}",
            captured.pixfmt(),
            dst_fmt.pixfmt
        )
        .into());
    }
    if src_width > dst_fmt.w || src_height > dst_fmt.h {
        return Err(format!(
            "src ({src_width}x{src_height}) > dst ({}x{})",
            dst_fmt.w, dst_fmt.h
        )
        .into());
    }
    if src_stride[0] < src_width * 4 {
        return Err(format!("stride {} < width*4 {}", src_stride[0], src_width * 4).into());
    }

    let dst_stride_y = dst_fmt.stride[0];
    let dst_stride_uv = dst_fmt.stride[1];
    dst.resize(dst_fmt.h * dst_stride_y * 2, 0);
    let dst_y = dst.as_mut_ptr();
    let dst_u = dst[dst_fmt.u..].as_mut_ptr();
    let dst_v = dst[dst_fmt.v..].as_mut_ptr();

    call_yuv!(ARGBToI420(
        src.as_ptr(),
        src_stride[0] as _,
        dst_y,
        dst_stride_y as _,
        dst_u,
        dst_stride_uv as _,
        dst_v,
        dst_stride_uv as _,
        src_width as _,
        src_height as _,
    ));

    Ok(())
}
