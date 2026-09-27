#![crate_type = "dylib"]
// https://github.com/microsoft/windows-rs/issues/1506
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// TODO: Update windows-rs
#![allow(unused_must_use)]
#![allow(non_snake_case)]

use std::{cell::RefCell, io::Read, rc::Rc};
use windows::core::{GUID, Interface, implement};

mod registry;
mod winstream;
use winstream::WinStream;

use windows as Windows;
use windows::Win32::{
    Foundation::*,
    Graphics::Imaging::*,
    System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IStream},
};

use jxl::{
    api::{
        Endianness, JxlBasicInfo, JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions,
        JxlOutputBuffer, JxlPixelFormat, ProcessingResult, states::Initialized,
    },
    headers::extra_channels::ExtraChannel,
};

mod dll;
mod guid;
mod runner;
use runner::RayonParallelRunner;

pub mod properties;
pub use properties::JXLPropertyStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WicPixelFormat {
    Gray16,
    Rgb48,
    Rgba64,
}

pub struct DecodedResult {
    raw_data: Vec<u8>,
    basic_info: JxlBasicInfo,
    color_type: JxlColorType,
    frame_count: usize,
    icc: Rc<Vec<u8>>,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
pub struct FrameBuffer {
    pub channels: usize,
    pub buf: Vec<u16>,
}

impl FrameBuffer {
    pub fn new(width: usize, height: usize, channels: usize) -> Self {
        Self {
            channels,
            buf: vec![0u16; width * height * channels],
        }
    }
}

#[implement(Windows::Win32::Graphics::Imaging::IWICBitmapDecoder)]
#[derive(Default)]
pub struct JXLWICBitmapDecoder {
    decoded: RefCell<Option<DecodedResult>>,
}

impl JXLWICBitmapDecoder {
    pub const CLSID: GUID = GUID::from_u128(0x655896c6_b7d0_4d74_8afb_a02ece3f5e5a);
    pub const CONTAINER_ID: GUID = GUID::from_u128(0x81e337bc_c1d1_4dee_a17c_402041ba9b5e);
}

impl IWICBitmapDecoder_Impl for JXLWICBitmapDecoder_Impl {
    fn QueryCapability(
        &self,
        _pistream: windows::core::Ref<'_, IStream>,
    ) -> windows::core::Result<u32> {
        log::trace!("QueryCapability");
        Ok((WICBitmapDecoderCapabilityCanDecodeSomeImages.0
            | WICBitmapDecoderCapabilityCanDecodeAllImages.0) as u32)
    }

    fn Initialize(
        &self,
        pistream: windows::core::Ref<'_, IStream>,
        _cacheoptions: WICDecodeOptions,
    ) -> windows::core::Result<()> {
        log::trace!("JXLWICBitmapDecoder::Initialize");

        let pistream = pistream.ok()?;
        let stream = WinStream::from(pistream);
        let mut stream = stream;
        let mut raw_data = Vec::new();
        stream.read_to_end(&mut raw_data).map_err(|err| {
            windows::core::Error::new(WINCODEC_ERR_BADIMAGE, format!("{:?}", err))
        })?;

        let mut input = &raw_data[..];
        let decoder = JxlDecoder::<Initialized>::new(JxlDecoderOptions::default());
        let decoder_with_info = match decoder.process(&mut input, Some(&mut RayonParallelRunner)) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                return Err(windows::core::Error::new(
                    WINCODEC_ERR_BADIMAGE,
                    "Unexpected EOF reading JXL header",
                ));
            }
            Err(err) => {
                return Err(windows::core::Error::new(
                    WINCODEC_ERR_BADIMAGE,
                    format!("{:?}", err),
                ));
            }
        };

        let basic_info = decoder_with_info.basic_info().clone();
        let (width, height) = (basic_info.size.0 as u32, basic_info.size.1 as u32);

        let icc = decoder_with_info
            .output_color_profile()
            .try_as_icc()
            .or_else(|| decoder_with_info.embedded_color_profile().try_as_icc())
            .map(|cow| cow.into_owned())
            .unwrap_or_default();

        let color_type = decoder_with_info.current_pixel_format().color_type;

        let frame_count = if basic_info.animation.is_some() {
            let mut scan_input = &raw_data[..];
            let mut options = JxlDecoderOptions::default();
            options.scan_frames_only = true;
            let scan_decoder = JxlDecoder::<Initialized>::new(options);
            if let Ok(ProcessingResult::Complete {
                result: mut dec_info,
            }) = scan_decoder.process(&mut scan_input, None)
            {
                let mut count = 0;
                while dec_info.has_more_frames() {
                    match dec_info.process(&mut scan_input, None) {
                        Ok(ProcessingResult::Complete { result: dec_frame }) => {
                            count += 1;
                            match dec_frame.skip_frame(&mut scan_input) {
                                Ok(ProcessingResult::Complete { result: next_info }) => {
                                    dec_info = next_info;
                                }
                                _ => break,
                            }
                        }
                        _ => break,
                    }
                }
                count.max(1)
            } else {
                1
            }
        } else {
            1
        };

        self.decoded.replace(Some(DecodedResult {
            raw_data,
            basic_info,
            color_type,
            frame_count,
            icc: Rc::new(icc),
            width,
            height,
        }));

        Ok(())
    }

    fn GetContainerFormat(&self) -> windows::core::Result<GUID> {
        log::trace!("JXLWICBitmapDecoder::GetContainerFormat");
        // Randomly generated
        Ok(JXLWICBitmapDecoder::CONTAINER_ID)
    }

    fn GetDecoderInfo(&self) -> windows::core::Result<IWICBitmapDecoderInfo> {
        log::trace!("JXLWICBitmapDecoder::GetDecoderInfo");
        unsafe {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let component_info = factory.CreateComponentInfo(&JXLWICBitmapDecoder::CLSID)?;
            component_info.cast()
        }
    }

    fn CopyPalette(
        &self,
        _pipalette: windows::core::Ref<'_, IWICPalette>,
    ) -> windows::core::Result<()> {
        log::trace!("JXLWICBitmapDecoder::CopyPalette");
        WINCODEC_ERR_PALETTEUNAVAILABLE.ok()
    }

    fn GetMetadataQueryReader(&self) -> windows::core::Result<IWICMetadataQueryReader> {
        log::trace!("JXLWICBitmapDecoder::GetMetadataQueryReader");
        Err(WINCODEC_ERR_UNSUPPORTEDOPERATION.into())
    }

    fn GetPreview(&self) -> windows::core::Result<IWICBitmapSource> {
        log::trace!("JXLWICBitmapDecoder::GetPreview");
        Err(WINCODEC_ERR_UNSUPPORTEDOPERATION.into())
    }

    fn GetColorContexts(
        &self,
        ccount: u32,
        ppicolorcontexts: *mut Option<IWICColorContext>,
        pcactualcount: *mut u32,
    ) -> windows::core::Result<()> {
        let decoded_ref = self.decoded.borrow();

        let Some(decoded) = decoded_ref.as_ref() else {
            return WINCODEC_ERR_NOTINITIALIZED.ok();
        };

        log::trace!(
            "JXLWICBitmapDecoder::GetColorContexts {} {:?} {:?}",
            ccount,
            ppicolorcontexts,
            pcactualcount
        );
        unsafe {
            if let Some(context) = ppicolorcontexts.as_mut()
                && ccount >= 1
                && !decoded.icc.is_empty()
                && let Some(context) = context.as_mut()
            {
                context.InitializeFromMemory(&decoded.icc[..])?;
            }
            if !pcactualcount.is_null() {
                *pcactualcount = if decoded.icc.is_empty() { 0 } else { 1 };
            }
        }
        Ok(())
    }

    fn GetThumbnail(&self) -> windows::core::Result<IWICBitmapSource> {
        log::trace!("JXLWICBitmapDecoder::GetThumbnail");
        Err(WINCODEC_ERR_CODECNOTHUMBNAIL.into())
    }

    fn GetFrameCount(&self) -> windows::core::Result<u32> {
        let decoded_ref = self.decoded.borrow();
        let Some(decoded) = decoded_ref.as_ref() else {
            return Err(WINCODEC_ERR_NOTINITIALIZED.into());
        };
        let frame_count = decoded.frame_count;

        log::trace!("JXLWICBitmapDecoder::GetFrameCount: {}", frame_count);
        Ok(frame_count as u32)
    }

    fn GetFrame(&self, index: u32) -> windows::core::Result<IWICBitmapFrameDecode> {
        let decoded_ref = self.decoded.borrow();
        let Some(decoded) = decoded_ref.as_ref() else {
            return Err(WINCODEC_ERR_NOTINITIALIZED.into());
        };

        log::trace!("[{}/{}]", index, decoded.frame_count);

        if index >= decoded.frame_count as u32 {
            return Err(WINCODEC_ERR_FRAMEMISSING.into());
        }

        let mut input = &decoded.raw_data[..];
        let decoder = JxlDecoder::<Initialized>::new(JxlDecoderOptions::default());
        let mut decoder_with_info =
            match decoder.process(&mut input, Some(&mut RayonParallelRunner)) {
                Ok(ProcessingResult::Complete { result }) => result,
                _ => return Err(WINCODEC_ERR_BADIMAGE.into()),
            };

        let main_alpha_channel = decoded
            .basic_info
            .extra_channels
            .iter()
            .position(|x| x.ec_type == ExtraChannel::Alpha);
        let has_alpha = main_alpha_channel.is_some();

        let (target_color_type, wic_format, channels) = if has_alpha {
            (JxlColorType::Rgba, WicPixelFormat::Rgba64, 4)
        } else if decoded.color_type.is_grayscale() {
            (JxlColorType::Grayscale, WicPixelFormat::Gray16, 1)
        } else {
            (JxlColorType::Rgb, WicPixelFormat::Rgb48, 3)
        };

        let current_format = decoder_with_info.current_pixel_format().clone();

        let new_format = JxlPixelFormat {
            color_type: target_color_type,
            color_data_format: Some(JxlDataFormat::U16 {
                endianness: Endianness::native(),
                bit_depth: 16,
            }),
            extra_channel_format: current_format
                .extra_channel_format
                .iter()
                .enumerate()
                .map(|(c, f)| {
                    if has_alpha && Some(c) == main_alpha_channel {
                        None
                    } else {
                        f.as_ref().map(|_| JxlDataFormat::U16 {
                            endianness: Endianness::native(),
                            bit_depth: 16,
                        })
                    }
                })
                .collect(),
        };
        decoder_with_info.set_pixel_format(new_format);

        let mut decoder_with_frame =
            match decoder_with_info.process(&mut input, Some(&mut RayonParallelRunner)) {
                Ok(ProcessingResult::Complete { result }) => result,
                _ => return Err(WINCODEC_ERR_FRAMEMISSING.into()),
            };

        for _ in 0..index {
            decoder_with_frame = match decoder_with_frame.skip_frame(&mut input) {
                Ok(ProcessingResult::Complete { result }) => {
                    match result.process(&mut input, Some(&mut RayonParallelRunner)) {
                        Ok(ProcessingResult::Complete { result }) => result,
                        _ => return Err(WINCODEC_ERR_FRAMEMISSING.into()),
                    }
                }
                _ => return Err(WINCODEC_ERR_FRAMEMISSING.into()),
            };
        }

        let width = decoded.width as usize;
        let height = decoded.height as usize;
        let bytes_per_sample = 2; // 16-bit
        let bytes_per_row = width * channels * bytes_per_sample;
        let mut u16_buf = vec![0u16; width * height * channels];

        {
            let byte_ptr = u16_buf.as_mut_ptr() as *mut u8;
            let mut out_buffers = [unsafe {
                JxlOutputBuffer::new_from_ptr(byte_ptr, height, bytes_per_row, bytes_per_row)
            }];
            match decoder_with_frame.process(
                &mut input,
                &mut out_buffers,
                Some(&mut RayonParallelRunner),
            ) {
                Ok(ProcessingResult::Complete { .. }) => {}
                _ => return Err(WINCODEC_ERR_FRAMEMISSING.into()),
            }
        }

        let fb = FrameBuffer {
            channels,
            buf: u16_buf,
        };

        let frame_decode = JXLWICBitmapFrameDecode::new(
            fb,
            wic_format,
            decoded.icc.clone(),
            decoded.width,
            decoded.height,
        );
        Ok(frame_decode.into())
    }
}

#[implement(Windows::Win32::Graphics::Imaging::IWICBitmapFrameDecode)]
pub struct JXLWICBitmapFrameDecode {
    frame: FrameBuffer,
    pixel_format: WicPixelFormat,
    icc: Rc<Vec<u8>>,
    width: u32,
    height: u32,
}

impl JXLWICBitmapFrameDecode {
    pub fn new(
        frame: FrameBuffer,
        pixel_format: WicPixelFormat,
        icc: Rc<Vec<u8>>,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            frame,
            pixel_format,
            icc,
            width,
            height,
        }
    }
}

#[allow(non_snake_case)]
#[allow(clippy::missing_safety_doc)]
impl IWICBitmapSource_Impl for JXLWICBitmapFrameDecode_Impl {
    fn GetSize(&self, puiwidth: *mut u32, puiheight: *mut u32) -> windows::core::Result<()> {
        log::trace!(
            "JXLWICBitmapFrameDecode::GetSize {}x{}",
            self.width,
            self.height
        );
        if puiwidth.is_null() || puiheight.is_null() {
            return Err(E_INVALIDARG.into());
        }
        unsafe {
            *puiwidth = self.width;
            *puiheight = self.height;
        }
        Ok(())
    }

    fn GetPixelFormat(&self) -> windows::core::Result<GUID> {
        log::trace!("JXLWICBitmapFrameDecode::GetPixelFormat");

        match self.pixel_format {
            WicPixelFormat::Gray16 => Ok(GUID_WICPixelFormat16bppGray),
            WicPixelFormat::Rgb48 => Ok(GUID_WICPixelFormat48bppRGB),
            WicPixelFormat::Rgba64 => Ok(GUID_WICPixelFormat64bppRGBA),
        }
    }

    fn GetResolution(&self, pdpix: *mut f64, pdpiy: *mut f64) -> windows::core::Result<()> {
        log::trace!("JXLWICBitmapFrameDecode::GetResolution");
        unsafe {
            *pdpix = 96f64;
            *pdpiy = 96f64;
        }
        Ok(())
    }

    fn CopyPalette(
        &self,
        _pipalette: windows::core::Ref<'_, IWICPalette>,
    ) -> windows::core::Result<()> {
        log::trace!("JXLWICBitmapFrameDecode::CopyPalette");
        WINCODEC_ERR_PALETTEUNAVAILABLE.ok()
    }

    fn CopyPixels(
        &self,
        prc: *const WICRect,
        cbstride: u32,
        _cbbuffersize: u32,
        pbbuffer: *mut u8,
    ) -> windows::core::Result<()> {
        log::trace!("JXLWICBitmapFrameDecode::CopyPixels");

        if pbbuffer.is_null() {
            return Err(E_INVALIDARG.into());
        }

        let full_rect = WICRect {
            X: 0,
            Y: 0,
            Width: self.width as i32,
            Height: self.height as i32,
        };

        let prc = if prc.is_null() {
            &full_rect
        } else {
            unsafe { &*prc }
        };

        log::trace!("JXLWICBitmapFrameDecode::CopyPixels::WICRect {:?}", prc);

        if prc.X < 0
            || prc.Y < 0
            || prc.Width <= 0
            || prc.Height <= 0
            || (prc.X + prc.Width) as u32 > self.width
            || (prc.Y + prc.Height) as u32 > self.height
        {
            return Err(E_INVALIDARG.into());
        }

        let channels = self.frame.channels;
        let buf = &self.frame.buf;
        let bytes_per_pixel = channels * 2;

        for y in prc.Y..(prc.Y + prc.Height) {
            let src_offset = ((self.width as i32 * y + prc.X) * (channels as i32)) as usize * 2;
            let dst_offset = ((y - prc.Y) as u32 * cbstride) as usize;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (buf.as_ptr() as *const u8).add(src_offset),
                    pbbuffer.add(dst_offset),
                    prc.Width as usize * bytes_per_pixel,
                );
            }
        }

        Ok(())
    }
}

impl IWICBitmapFrameDecode_Impl for JXLWICBitmapFrameDecode_Impl {
    fn GetMetadataQueryReader(&self) -> windows::core::Result<IWICMetadataQueryReader> {
        log::trace!("JXLWICBitmapFrameDecode::GetMetadataQueryReader");
        Err(WINCODEC_ERR_UNSUPPORTEDOPERATION.into())
    }

    fn GetColorContexts(
        &self,
        ccount: u32,
        ppicolorcontexts: *mut Option<IWICColorContext>,
        pcactualcount: *mut u32,
    ) -> windows::core::Result<()> {
        log::trace!(
            "JXLWICBitmapFrameDecode::GetColorContexts {} {:?} {:?}",
            ccount,
            ppicolorcontexts,
            pcactualcount
        );
        unsafe {
            if let Some(context) = ppicolorcontexts.as_mut()
                && ccount >= 1
                && !self.icc.is_empty()
                && let Some(context) = context.as_mut()
            {
                context.InitializeFromMemory(&self.icc[..])?;
            }
            if !pcactualcount.is_null() {
                *pcactualcount = if self.icc.is_empty() { 0 } else { 1 };
            }
        }
        Ok(())
    }

    fn GetThumbnail(&self) -> windows::core::Result<IWICBitmapSource> {
        log::trace!("JXLWICBitmapFrameDecode::GetThumbnail");
        Err(WINCODEC_ERR_CODECNOTHUMBNAIL.into())
    }
}
