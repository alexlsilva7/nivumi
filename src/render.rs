#[cfg(debug_assertions)]
#[path = "frame_capture.rs"]
pub mod frame_capture;
use crate::theme;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::Common::*, Direct2D::*, Direct3D::*, Direct3D11::*, DirectComposition::*,
            DirectWrite::*, Dxgi::Common::*, Dxgi::*, Imaging::*,
        },
        System::Com::*,
        UI::Shell::SHCreateMemStream,
    },
};
use windows_numerics::Vector2;

pub fn color(rgb: u32) -> D2D1_COLOR_F {
    color_alpha(rgb, 1.)
}

pub fn color_alpha(rgb: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 255) as f32 / 255.,
        g: ((rgb >> 8) & 255) as f32 / 255.,
        b: (rgb & 255) as f32 / 255.,
        a,
    }
}

pub struct Renderer {
    svg_icons: std::cell::RefCell<std::collections::HashMap<(String, u32), ID2D1SvgDocument>>,
    pub dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _composition: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    _device: ID3D11Device,
    write: IDWriteFactory,
    brush: ID2D1SolidColorBrush,
    dash_style: Option<ID2D1StrokeStyle1>,
    pub width: f32,
    pub height: f32,
    pub dpi: f32,
    thumbnails: std::cell::RefCell<std::collections::HashMap<u64, ID2D1Bitmap1>>,
    cover: std::cell::RefCell<Option<(u64, ID2D1Bitmap1)>>,
    prev_cover: std::cell::RefCell<Option<(ID2D1Bitmap1, std::time::Instant)>>,
}
impl Renderer {
    /// # Safety
    /// Call between begin/end on the renderer's owning UI thread.
    pub unsafe fn svg_icon(&self, source: &str, x: f32, y: f32, size: f32, color: u32) -> bool {
        let Ok(dc) = self.dc.cast::<ID2D1DeviceContext5>() else {
            return false;
        };
        let key = (source.to_owned(), color);
        let mut cache = self.svg_icons.borrow_mut();
        if !cache.contains_key(&key) {
            let xml = source.replace("currentColor", &format!("#{color:06x}"));
            let Some(stream) = SHCreateMemStream(Some(xml.as_bytes())) else {
                return false;
            };
            let Ok(document) = dc.CreateSvgDocument(
                &stream,
                D2D_SIZE_F {
                    width: 24.,
                    height: 24.,
                },
            ) else {
                return false;
            };
            cache.insert(key.clone(), document);
        }
        let mut original = windows_numerics::Matrix3x2::identity();
        self.dc.GetTransform(&mut original);
        let mut transform = original;
        let scale = size / 24.;
        transform.M11 *= scale;
        transform.M12 *= scale;
        transform.M21 *= scale;
        transform.M22 *= scale;
        transform.M31 = original.M31 + x * original.M11 + y * original.M21;
        transform.M32 = original.M32 + x * original.M12 + y * original.M22;
        self.dc.SetTransform(&transform);
        dc.DrawSvgDocument(&cache[&key]);
        self.dc.SetTransform(&original);
        true
    }

    /// # Safety
    /// Call inside a drawing frame on the owning UI thread.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn notch(
        &self,
        x: f32,
        w: f32,
        h: f32,
        r: f32,
        ear: f32,
        accent: [f32; 3],
        wave: Option<f32>,
    ) -> Option<ID2D1PathGeometry> {
        let accent = theme::artwork_accent(accent);
        let draw = (|| -> Result<ID2D1PathGeometry> {
            let path = self.dc.GetFactory()?.CreatePathGeometry()?;
            let sink = path.Open()?;
            let p = |x, y| Vector2 { X: x, Y: y };
            sink.BeginFigure(p(x - ear, 0.), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: p(x - ear * 0.45, 0.),
                point2: p(x, ear * 0.45),
                point3: p(x, ear),
            });
            sink.AddLine(p(x, h - r));
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: p(x, h - r * 0.45),
                point2: p(x + r * 0.45, h),
                point3: p(x + r, h),
            });
            sink.AddLine(p(x + w - r, h));
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: p(x + w - r * 0.45, h),
                point2: p(x + w, h - r * 0.45),
                point3: p(x + w, h - r),
            });
            sink.AddLine(p(x + w, ear));
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: p(x + w, ear * 0.45),
                point2: p(x + w + ear * 0.45, 0.),
                point3: p(x + w + ear, 0.),
            });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            self.brush.SetColor(&color(theme::NOTCH_SURFACE));
            self.dc.FillGeometry(&path, &self.brush, None);
            if accent.iter().any(|c| *c > 0.001) {
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.,
                        color: color(theme::NOTCH_SURFACE),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.32,
                        color: color(theme::NOTCH_SURFACE),
                    },
                    D2D1_GRADIENT_STOP {
                        position: 0.68,
                        color: D2D1_COLOR_F {
                            r: accent[0] * 0.3,
                            g: accent[1] * 0.3,
                            b: accent[2] * 0.3,
                            a: 1.,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.,
                        color: D2D1_COLOR_F {
                            r: accent[0],
                            g: accent[1],
                            b: accent[2],
                            a: 1.,
                        },
                    },
                ];
                if let Ok(collection) = self.dc.CreateGradientStopCollection(
                    &stops,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    D2D1_EXTEND_MODE_CLAMP,
                    D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
                ) {
                    if let Ok(brush) = self.dc.CreateLinearGradientBrush(
                        &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                            startPoint: p(x, 0.),
                            endPoint: p(x, h),
                        },
                        None,
                        &collection,
                    ) {
                        self.dc.FillGeometry(&path, &brush, None);
                    }
                }
            }
            if let Some(phase) = wave.filter(|_| accent.iter().any(|color| *color > 0.001)) {
                let stops = [
                    D2D1_GRADIENT_STOP {
                        position: 0.,
                        color: D2D1_COLOR_F {
                            r: (accent[0] * 1.6).min(1.),
                            g: (accent[1] * 1.6).min(1.),
                            b: (accent[2] * 1.6).min(1.),
                            a: 0.32,
                        },
                    },
                    D2D1_GRADIENT_STOP {
                        position: 1.,
                        color: D2D1_COLOR_F {
                            r: 0.,
                            g: 0.,
                            b: 0.,
                            a: 0.,
                        },
                    },
                ];
                if let Ok(collection) = self.dc.CreateGradientStopCollection(
                    &stops,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    D2D1_EXTEND_MODE_CLAMP,
                    D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
                ) {
                    if let Ok(brush) = self.dc.CreateRadialGradientBrush(
                        &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                            center: p(
                                x + w * (0.5 + 0.38 * (phase * 0.65).sin()),
                                h * (0.96 + 0.08 * (phase * 0.8).sin()),
                            ),
                            gradientOriginOffset: p(0., 0.),
                            radiusX: w * 0.55,
                            radiusY: h * 0.9,
                        },
                        None,
                        &collection,
                    ) {
                        self.dc.FillGeometry(&path, &brush, None);
                    }
                }
            }
            Ok(path)
        })();
        match draw {
            Ok(path) => Some(path),
            Err(_) => {
                self.fill(x, 0., w, h, r, 0);
                self.fill(x, 0., w, ear, 0., 0);
                None
            }
        }
    }
    /// # Safety
    /// Call inside a frame on the renderer's owning UI thread.
    pub unsafe fn fill_path(&self, path: &ID2D1PathGeometry, rgb: u32, opacity: f32) {
        self.brush.SetColor(&color_alpha(rgb, opacity));
        self.dc.FillGeometry(path, &self.brush, None);
    }
    /// # Safety
    /// Pair with pop_content inside a frame on the owning UI thread.
    pub unsafe fn push_content(&self, opacity: f32, mask: Option<&ID2D1PathGeometry>) {
        let mut parameters = D2D1_LAYER_PARAMETERS1 {
            contentBounds: D2D_RECT_F {
                left: 0.,
                top: 0.,
                right: self.width,
                bottom: self.height,
            },
            geometricMask: std::mem::ManuallyDrop::new(mask.and_then(|p| p.cast().ok())),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: windows_numerics::Matrix3x2::identity(),
            opacity,
            ..Default::default()
        };
        self.dc.PushLayer(&parameters, None);
        // windows-rs uses ManuallyDrop for COM fields in FFI structs.
        std::mem::ManuallyDrop::drop(&mut parameters.geometricMask);
    }
    /// # Safety
    /// Must match one push_content on the owning UI thread.
    pub unsafe fn pop_content(&self) {
        self.dc.PopLayer();
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn new(hwnd: HWND, width: u32, height: u32, dpi: f32) -> Result<Self> {
        let mut device = None;
        let make = |driver, output: &mut Option<ID3D11Device>| {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(output),
                None,
                None,
            )
        };
        if make(D3D_DRIVER_TYPE_HARDWARE, &mut device).is_err() {
            make(D3D_DRIVER_TYPE_WARP, &mut device)?;
        }
        let device = device.ok_or_else(Error::from_thread)?;
        let dxgi: IDXGIDevice = device.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
        let swap = factory.CreateSwapChainForComposition(
            &device,
            &DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            },
            None,
        )?;
        let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dc = d2d
            .CreateDevice(&dxgi)?
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let surface: IDXGISurface = swap.GetBuffer(0)?;
        let bitmap = dc.CreateBitmapFromDxgiSurface(
            &surface,
            Some(&D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi,
                dpiY: dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            }),
        )?;
        dc.SetTarget(&bitmap);
        dc.SetDpi(dpi, dpi);
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let target = composition.CreateTargetForHwnd(hwnd, true)?;
        let visual = composition.CreateVisual()?;
        visual.SetContent(&swap)?;
        target.SetRoot(&visual)?;
        composition.Commit()?;
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let brush = dc.CreateSolidColorBrush(&color(0xffffff), None)?;
        let stroke_props = D2D1_STROKE_STYLE_PROPERTIES1 {
            dashStyle: D2D1_DASH_STYLE_DASH,
            ..Default::default()
        };
        let dash_style = d2d.CreateStrokeStyle(&stroke_props, None).ok();
        Ok(Self {
            dc,
            swap,
            _composition: composition,
            _target: target,
            _visual: visual,
            _device: device,
            write,
            brush,
            dash_style,
            width: width as f32 * 96. / dpi,
            height: height as f32 * 96. / dpi,
            dpi,
            thumbnails: Default::default(),
            svg_icons: std::cell::RefCell::new(std::collections::HashMap::new()),
            cover: std::cell::RefCell::new(None),
            prev_cover: std::cell::RefCell::new(None),
        })
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn begin(&self) {
        self.dc.BeginDraw();
        self.dc.Clear(Some(&D2D1_COLOR_F {
            r: 0.,
            g: 0.,
            b: 0.,
            a: 0.,
        }));
    }
    pub fn is_crossfading(&self) -> bool {
        self.prev_cover
            .borrow()
            .as_ref()
            .is_some_and(|(_, start)| start.elapsed().as_secs_f32() < 0.200)
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn artwork(&self, bytes: &[u8], rect: D2D_RECT_F) -> bool {
        use std::hash::{Hash, Hasher};
        if bytes.is_empty() {
            return false;
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hash);
        let key = hash.finish();
        if self
            .cover
            .borrow()
            .as_ref()
            .is_none_or(|(old, _)| *old != key)
        {
            let decoded = (|| -> Result<ID2D1Bitmap1> {
                let stream = SHCreateMemStream(Some(bytes)).ok_or_else(Error::from_thread)?;
                let factory: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let decoder = factory.CreateDecoderFromStream(
                    &stream,
                    std::ptr::null(),
                    WICDecodeMetadataCacheOnLoad,
                )?;
                let frame = decoder.GetFrame(0)?;
                let converter = factory.CreateFormatConverter()?;
                converter.Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.,
                    WICBitmapPaletteTypeCustom,
                )?;
                self.dc.CreateBitmapFromWicBitmap(&converter, None)
            })();
            match decoded {
                Ok(bitmap) => {
                    if let Some((_, old_bmp)) = self.cover.borrow().as_ref() {
                        *self.prev_cover.borrow_mut() =
                            Some((old_bmp.clone(), std::time::Instant::now()));
                    }
                    *self.cover.borrow_mut() = Some((key, bitmap));
                }
                Err(_) => return false,
            }
        }
        if let Some((_, bitmap)) = self.cover.borrow().as_ref() {
            let mut crossfade_active = false;
            if let Some((prev_bmp, start)) = self.prev_cover.borrow().as_ref() {
                let elapsed = start.elapsed().as_secs_f32();
                if elapsed < 0.200 {
                    crossfade_active = true;
                    let progress = (elapsed / 0.200).clamp(0., 1.);
                    self.dc.DrawBitmap(
                        prev_bmp,
                        Some(&rect),
                        1.,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    );
                    self.dc.DrawBitmap(
                        bitmap,
                        Some(&rect),
                        progress,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    );
                }
            }
            if !crossfade_active {
                *self.prev_cover.borrow_mut() = None;
                self.dc.DrawBitmap(
                    bitmap,
                    Some(&rect),
                    1.,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    None,
                    None,
                );
            }
            return true;
        }
        false
    }
    /// # Safety
    /// Draw inside begin/end on the renderer's UI thread.
    pub unsafe fn thumbnail(
        &self,
        id: u64,
        image: &crate::shelf::Thumbnail,
        rect: D2D_RECT_F,
    ) -> bool {
        let mut cache = self.thumbnails.borrow_mut();
        if !cache.contains_key(&id) {
            if cache.len() >= 128 {
                cache.clear();
            }
            let bitmap = self.dc.CreateBitmap(
                D2D_SIZE_U {
                    width: image.width,
                    height: image.height,
                },
                Some(image.pixels.as_ptr().cast()),
                image.width * 4,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                    ..Default::default()
                },
            );
            let Ok(bitmap) = bitmap else {
                return false;
            };
            cache.insert(id, bitmap);
        }
        let scale = ((rect.right - rect.left) / image.width as f32)
            .min((rect.bottom - rect.top) / image.height as f32);
        let w = image.width as f32 * scale;
        let h = image.height as f32 * scale;
        let left = (rect.left + rect.right - w) / 2.;
        let top = (rect.top + rect.bottom - h) / 2.;
        self.dc.DrawBitmap(
            &cache[&id],
            Some(&D2D_RECT_F {
                left,
                top,
                right: left + w,
                bottom: top + h,
            }),
            1.,
            D2D1_INTERPOLATION_MODE_LINEAR,
            None,
            None,
        );
        true
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn end(&self) -> Result<()> {
        self.dc.EndDraw(None, None)?;
        #[cfg(debug_assertions)]
        frame_capture::capture(self)?;
        self.swap.Present(1, DXGI_PRESENT(0)).ok()
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn fill(&self, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: u32) {
        self.fill_alpha(x, y, w, h, r, rgb, 1.);
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn fill_alpha(&self, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: u32, a: f32) {
        self.brush.SetColor(&color_alpha(rgb, a));
        self.dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                radiusX: r,
                radiusY: r,
            },
            &self.brush,
        );
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn stroke_rounded_rect(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        rgb: u32,
        a: f32,
        stroke_width: f32,
        dashed: bool,
    ) {
        self.brush.SetColor(&color_alpha(rgb, a));
        let style: Option<ID2D1StrokeStyle> = if dashed {
            self.dash_style.as_ref().and_then(|s| s.cast().ok())
        } else {
            None
        };
        self.dc.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                radiusX: r,
                radiusY: r,
            },
            &self.brush,
            stroke_width,
            style.as_ref(),
        );
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn line(&self, x: f32, y: f32, x2: f32, y2: f32, rgb: u32, width: f32) {
        self.line_alpha(x, y, x2, y2, rgb, 1., width);
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn line_alpha(
        &self,
        x: f32,
        y: f32,
        x2: f32,
        y2: f32,
        rgb: u32,
        a: f32,
        width: f32,
    ) {
        self.brush.SetColor(&color_alpha(rgb, a));
        self.dc.DrawLine(
            Vector2 { X: x, Y: y },
            Vector2 { X: x2, Y: y2 },
            &self.brush,
            width,
            None,
        );
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn dashed_line(
        &self,
        x: f32,
        y: f32,
        x2: f32,
        y2: f32,
        rgb: u32,
        a: f32,
        width: f32,
    ) {
        self.brush.SetColor(&color_alpha(rgb, a));
        let style: Option<ID2D1StrokeStyle> = self.dash_style.as_ref().and_then(|s| s.cast().ok());
        self.dc.DrawLine(
            Vector2 { X: x, Y: y },
            Vector2 { X: x2, Y: y2 },
            &self.brush,
            width,
            style.as_ref(),
        );
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn circle(&self, x: f32, y: f32, r: f32, rgb: u32) {
        self.circle_alpha(x, y, r, rgb, 1.);
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn circle_alpha(&self, x: f32, y: f32, r: f32, rgb: u32, a: f32) {
        self.brush.SetColor(&color_alpha(rgb, a));
        self.dc.FillEllipse(
            &D2D1_ELLIPSE {
                point: Vector2 { X: x, Y: y },
                radiusX: r,
                radiusY: r,
            },
            &self.brush,
        );
    }
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn stroke_circle(
        &self,
        x: f32,
        y: f32,
        r: f32,
        rgb: u32,
        a: f32,
        stroke_width: f32,
    ) {
        self.brush.SetColor(&color_alpha(rgb, a));
        self.dc.DrawEllipse(
            &D2D1_ELLIPSE {
                point: Vector2 { X: x, Y: y },
                radiusX: r,
                radiusY: r,
            },
            &self.brush,
            stroke_width,
            None,
        );
    }
    unsafe fn wrapped_layout(
        &self,
        text: &str,
        width: f32,
        size: f32,
        bold: bool,
    ) -> Result<IDWriteTextLayout> {
        let format = self.write.CreateTextFormat(
            w!("Segoe UI"),
            None,
            if bold {
                DWRITE_FONT_WEIGHT_SEMI_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            },
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("pt-BR"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        self.write.CreateTextLayout(
            &text.encode_utf16().collect::<Vec<_>>(),
            &format,
            width.max(1.),
            100000.,
        )
    }
    /// # Safety
    /// Call on the owning rendering thread.
    pub unsafe fn text_height(&self, text: &str, width: f32, size: f32, bold: bool) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        self.wrapped_layout(text, width, size, bold)
            .and_then(|layout| layout.GetMetrics(&mut metrics))
            .map(|_| metrics.height)
            .unwrap_or(size * 1.5)
    }
    /// # Safety
    /// Call on the owning rendering thread.
    pub unsafe fn text_width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        self.wrapped_layout(text, 10000., size, bold)
            .and_then(|layout| layout.GetMetrics(&mut metrics))
            .map(|_| metrics.width)
            .unwrap_or_else(|_| text.len() as f32 * size * 0.6)
    }
    /// # Safety
    /// Call inside a drawing frame on the owning rendering thread.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn text_wrapped(
        &self,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        rgb: u32,
        bold: bool,
    ) {
        if let Ok(layout) = self.wrapped_layout(text, width, size, bold) {
            self.brush.SetColor(&color(rgb));
            self.dc.DrawTextLayout(
                Vector2 { X: x, Y: y },
                &layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }
    #[allow(clippy::too_many_arguments)]
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn text(
        &self,
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        rgb: u32,
        bold: bool,
        center: bool,
    ) {
        let weight = if bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let font_res = self
            .write
            .CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("pt-BR"),
            )
            .or_else(|_| {
                self.write.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("pt-BR"),
                )
            });
        if let Ok(format) = font_res {
            let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            if let Ok(ellipsis) = self.write.CreateEllipsisTrimmingSign(&format) {
                let _ = format.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    &ellipsis,
                );
            }
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            if center {
                let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            }
            self.brush.SetColor(&color(rgb));
            self.dc.DrawText(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                &D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                },
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
    #[allow(clippy::too_many_arguments)]
    /// # Safety
    /// Call on the owning UI STA with a live renderer and window. Drawing calls require begin/end pairing.
    pub unsafe fn text_marquee(
        &self,
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        rgb: u32,
        bold: bool,
        offset: f32,
        is_scrolling: bool,
    ) {
        if !is_scrolling {
            self.text(text, x, y, w, h, size, rgb, bold, false);
            return;
        }
        let weight = if bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let font_res = self
            .write
            .CreateTextFormat(
                w!("Segoe UI Variable Text"),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("pt-BR"),
            )
            .or_else(|_| {
                self.write.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("pt-BR"),
                )
            });
        if let Ok(format) = font_res {
            let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            self.brush.SetColor(&color(rgb));
            let clip = D2D_RECT_F {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            };
            self.dc
                .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.dc.DrawText(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                &D2D_RECT_F {
                    left: x - offset,
                    top: y,
                    right: x - offset + 10000.,
                    bottom: y + h,
                },
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.dc.PopAxisAlignedClip();
        }
    }
}
