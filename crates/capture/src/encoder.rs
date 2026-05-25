use crate::convert::{EncodeYuvFormat, Pixfmt};
use shared::EncodedPacket;
use std::os::raw::c_int;
use std::{ptr, slice};

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    improper_ctypes,
    dead_code,
    unused_imports
)]
mod vpx_ffi {
    include!(concat!(env!("OUT_DIR"), "/vpx_ffi.rs"));
}
use vp8e_enc_control_id::VP8E_SET_CPUUSED;
use vpx_ffi::*;

#[derive(Debug)]
pub enum VpxError {
    FailedCall(String),
    BadPtr(String),
}

impl std::fmt::Display for VpxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VpxError::FailedCall(message) => write!(f, "vpx call failed: {message}"),
            VpxError::BadPtr(message) => write!(f, "vpx bad pointer: {message}"),
        }
    }
}

impl std::error::Error for VpxError {}

pub type VpxResult<T> = std::result::Result<T, VpxError>;
pub type Vp8Error = VpxError;

macro_rules! call_vpx {
    ($x:expr) => {{
        let result = unsafe { $x };
        let result_int = unsafe { std::mem::transmute::<_, i32>(result) };
        if result_int != 0 {
            return Err(VpxError::FailedCall(format!(
                "errcode={} at {}:{}",
                result_int,
                file!(),
                line!()
            )));
        }
        result
    }};
}

macro_rules! call_vpx_ptr {
    ($x:expr) => {{
        let result = unsafe { $x };
        let result_int = unsafe { std::mem::transmute::<_, isize>(result) };
        if result_int == 0 {
            return Err(VpxError::BadPtr(format!(
                "null ptr at {}:{}",
                file!(),
                line!()
            )));
        }
        result
    }};
}

pub const STRIDE_ALIGN: usize = 64;

pub struct Vp8Encoder {
    ctx: vpx_codec_ctx_t,
    width: usize,
    height: usize,
    pub yuvfmt: EncodeYuvFormat,
}

#[derive(Clone, Copy, Debug)]
pub struct Vp8EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub quality: f32,
    pub keyframe_interval: Option<u32>,
    pub cpu_used: i32,
}

impl Vp8Encoder {
    pub fn new(config: Vp8EncoderConfig) -> VpxResult<Self> {
        let iface = call_vpx_ptr!(vpx_codec_vp8_cx());
        let mut enc_cfg = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
        call_vpx!(vpx_codec_enc_config_default(iface, &mut enc_cfg, 0));

        enc_cfg.g_w = config.width;
        enc_cfg.g_h = config.height;
        enc_cfg.g_timebase.num = 1;
        enc_cfg.g_timebase.den = 1000;
        enc_cfg.rc_dropframe_thresh = 25;
        enc_cfg.rc_undershoot_pct = 95;
        enc_cfg.g_threads = (num_cpus::get() / 2).clamp(1, 4) as _;
        enc_cfg.g_error_resilient = VPX_ERROR_RESILIENT_DEFAULT;
        enc_cfg.rc_end_usage = vpx_rc_mode::VPX_CBR;
        enc_cfg.rc_target_bitrate = Self::bitrate(config.width, config.height, config.quality);
        enc_cfg.g_profile = 0;

        let (q_min, q_max) = Self::calc_q_values(config.quality);
        enc_cfg.rc_min_quantizer = q_min;
        enc_cfg.rc_max_quantizer = q_max;

        if let Some(keyframe_interval) = config.keyframe_interval {
            enc_cfg.kf_min_dist = 0;
            enc_cfg.kf_max_dist = keyframe_interval as _;
        }

        let mut ctx = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
        call_vpx!(vpx_codec_enc_init_ver(
            &mut ctx,
            iface,
            &enc_cfg,
            0,
            VPX_ENCODER_ABI_VERSION as _
        ));
        call_vpx!(vpx_codec_control_(
            &mut ctx,
            VP8E_SET_CPUUSED as _,
            config.cpu_used.clamp(0, 16) as c_int,
        ));

        Ok(Self {
            ctx,
            width: config.width as _,
            height: config.height as _,
            yuvfmt: Self::get_yuvfmt(config.width, config.height),
        })
    }

    pub fn encode(&mut self, pts: i64, data: &[u8]) -> VpxResult<Vec<EncodedPacket>> {
        self.encode_with_flags(pts, data, 0)
    }

    pub fn force_keyframe(&mut self, pts: i64, data: &[u8]) -> VpxResult<Vec<EncodedPacket>> {
        self.encode_with_flags(pts, data, VPX_EFLAG_FORCE_KF as _)
    }

    pub fn flush(&mut self) -> VpxResult<Vec<EncodedPacket>> {
        call_vpx!(vpx_codec_encode(
            &mut self.ctx,
            ptr::null(),
            -1,
            1,
            0,
            VPX_DL_REALTIME as _,
        ));
        self.collect_packets()
    }

    pub fn set_quality(&mut self, quality: f32) -> VpxResult<()> {
        let enc_cfg = unsafe { self.ctx.config.enc };
        if enc_cfg.is_null() {
            return Err(VpxError::BadPtr("encoder config is null".to_owned()));
        }

        let mut enc_cfg = unsafe { *enc_cfg };
        let (q_min, q_max) = Self::calc_q_values(quality);
        enc_cfg.rc_min_quantizer = q_min;
        enc_cfg.rc_max_quantizer = q_max;
        enc_cfg.rc_target_bitrate = Self::bitrate(self.width as _, self.height as _, quality);
        call_vpx!(vpx_codec_enc_config_set(&mut self.ctx, &enc_cfg));
        Ok(())
    }

    pub fn get_yuvfmt(width: u32, height: u32) -> EncodeYuvFormat {
        let mut img = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
        unsafe {
            vpx_img_wrap(
                &mut img,
                vpx_img_fmt::VPX_IMG_FMT_I420,
                width as _,
                height as _,
                STRIDE_ALIGN as _,
                0x1 as _,
            );
        }
        EncodeYuvFormat {
            pixfmt: Pixfmt::I420,
            w: img.w as _,
            h: img.h as _,
            stride: img.stride.map(|stride| stride as usize).to_vec(),
            u: img.planes[1] as usize - img.planes[0] as usize,
            v: img.planes[2] as usize - img.planes[0] as usize,
        }
    }

    fn encode_with_flags(
        &mut self,
        pts: i64,
        data: &[u8],
        flags: vpx_enc_frame_flags_t,
    ) -> VpxResult<Vec<EncodedPacket>> {
        if data.len() < self.width * self.height * 12 / 8 {
            return Err(VpxError::FailedCall("len not enough".to_owned()));
        }

        let mut image = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
        call_vpx_ptr!(vpx_img_wrap(
            &mut image,
            vpx_img_fmt::VPX_IMG_FMT_I420,
            self.width as _,
            self.height as _,
            STRIDE_ALIGN as _,
            data.as_ptr() as _,
        ));
        call_vpx!(vpx_codec_encode(
            &mut self.ctx,
            &image,
            pts as _,
            1,
            flags,
            VPX_DL_REALTIME as _,
        ));
        self.collect_packets()
    }

    fn collect_packets(&mut self) -> VpxResult<Vec<EncodedPacket>> {
        let mut packets = Vec::new();
        let mut iter: vpx_codec_iter_t = ptr::null();

        loop {
            let pkt = unsafe { vpx_codec_get_cx_data(&mut self.ctx, &mut iter) };
            if pkt.is_null() {
                break;
            }
            if unsafe { (*pkt).kind } != vpx_codec_cx_pkt_kind::VPX_CODEC_CX_FRAME_PKT {
                continue;
            }

            let frame = unsafe { &(*pkt).data.frame };
            let data = unsafe { slice::from_raw_parts(frame.buf as _, frame.sz as _) }.to_vec();
            packets.push(EncodedPacket {
                data,
                is_keyframe: (frame.flags & VPX_FRAME_IS_KEY) != 0,
                pts: frame.pts,
            });
        }

        Ok(packets)
    }

    fn bitrate(width: u32, height: u32, quality: f32) -> u32 {
        let bitrate = base_bitrate(width, height) as f32;
        (bitrate * quality.clamp(0.0, 2.0)) as u32
    }

    fn calc_q_values(quality: f32) -> (u32, u32) {
        let b = (quality.clamp(0.0, 2.0) * 100.0) as u32;
        let q_min1 = 36;
        let q_min2 = 0;
        let q_max1 = 56;
        let q_max2 = 37;
        let t = b as f32 / 200.0;
        let q_min = ((1.0 - t) * q_min1 as f32 + t * q_min2 as f32).round() as u32;
        let q_max = ((1.0 - t) * q_max1 as f32 + t * q_max2 as f32).round() as u32;

        (q_min.clamp(q_min2, q_min1), q_max.clamp(q_max2, q_max1))
    }
}

impl Drop for Vp8Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = vpx_codec_destroy(&mut self.ctx);
        }
    }
}

pub fn base_bitrate(width: u32, height: u32) -> u32 {
    const RESOLUTION_PRESETS: &[(u32, u32, u32)] = &[
        (640, 480, 400),
        (800, 600, 500),
        (1024, 768, 800),
        (1280, 720, 1000),
        (1366, 768, 1100),
        (1440, 900, 1300),
        (1600, 900, 1500),
        (1920, 1080, 2073),
        (2048, 1080, 2200),
        (2560, 1440, 3000),
        (3440, 1440, 4000),
        (3840, 2160, 5000),
        (7680, 4320, 12000),
    ];
    let pixels = width * height;

    let (preset_pixels, preset_bitrate) = RESOLUTION_PRESETS
        .iter()
        .map(|(w, h, bitrate)| (w * h, bitrate))
        .min_by_key(|(preset_pixels, _)| {
            if *preset_pixels >= pixels {
                preset_pixels - pixels
            } else {
                pixels - preset_pixels
            }
        })
        .unwrap_or(((1920 * 1080) as u32, &2073));

    (*preset_bitrate as f32 * (pixels as f32 / preset_pixels as f32)).round() as u32
}

unsafe impl Send for vpx_codec_ctx_t {}
