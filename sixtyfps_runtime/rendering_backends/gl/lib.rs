/* LICENSE BEGIN
    This file is part of the SixtyFPS Project -- https://sixtyfps.io
    Copyright (c) 2020 Olivier Goffart <olivier.goffart@sixtyfps.io>
    Copyright (c) 2020 Simon Hausmann <simon.hausmann@sixtyfps.io>

    SPDX-License-Identifier: GPL-3.0-only
    This file is also available under commercial licensing terms.
    Please contact info@sixtyfps.io for more information.
LICENSE END */

use sixtyfps_corelib::items::Item;
use sixtyfps_corelib::{eventloop::ComponentWindow, items::ItemRenderer};
use sixtyfps_corelib::{
    graphics::{Color, GraphicsBackend, GraphicsWindow, Rect, Resource},
    SharedVector,
};

pub struct GLRenderer {
    canvas: Option<femtovg::Canvas<femtovg::renderer::OpenGl>>,

    #[cfg(target_arch = "wasm32")]
    window: Rc<winit::window::Window>,
    #[cfg(target_arch = "wasm32")]
    event_loop_proxy:
        Rc<winit::event_loop::EventLoopProxy<sixtyfps_corelib::eventloop::CustomEvent>>,
    #[cfg(not(target_arch = "wasm32"))]
    windowed_context: Option<glutin::WindowedContext<glutin::NotCurrent>>,
}

impl GLRenderer {
    pub fn new(
        event_loop: &winit::event_loop::EventLoop<sixtyfps_corelib::eventloop::CustomEvent>,
        window_builder: winit::window::WindowBuilder,
        #[cfg(target_arch = "wasm32")] canvas_id: &str,
    ) -> GLRenderer {
        #[cfg(not(target_arch = "wasm32"))]
        let (windowed_context, renderer) = {
            let windowed_context = glutin::ContextBuilder::new()
                .with_vsync(true)
                .build_windowed(window_builder, &event_loop)
                .unwrap();
            let windowed_context = unsafe { windowed_context.make_current().unwrap() };

            let renderer = femtovg::renderer::OpenGl::new(|symbol| {
                windowed_context.get_proc_address(symbol) as *const _
            })
            .unwrap();

            #[cfg(target_os = "macos")]
            {
                use cocoa::appkit::NSView;
                use winit::platform::macos::WindowExtMacOS;
                let ns_view = windowed_context.window().ns_view();
                let view_id: cocoa::base::id = ns_view as *const _ as *mut _;
                unsafe {
                    NSView::setLayerContentsPlacement(view_id, cocoa::appkit::NSViewLayerContentsPlacement::NSViewLayerContentsPlacementTopLeft)
                }
            }

            (windowed_context, renderer)
        };

        #[cfg(target_arch = "wasm32")]
        let event_loop_proxy = Rc::new(event_loop.create_proxy());

        #[cfg(target_arch = "wasm32")]
        let (window, renderer) = {
            let canvas = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .get_element_by_id(canvas_id)
                .unwrap()
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .unwrap();

            use winit::platform::web::WindowBuilderExtWebSys;
            use winit::platform::web::WindowExtWebSys;

            let existing_canvas_size = winit::dpi::LogicalSize::new(
                canvas.client_width() as u32,
                canvas.client_height() as u32,
            );

            let window =
                Rc::new(window_builder.with_canvas(Some(canvas)).build(&event_loop).unwrap());

            // Try to maintain the existing size of the canvas element. A window created with winit
            // on the web will always have 1024x768 as size otherwise.

            let resize_canvas = {
                let event_loop_proxy = event_loop_proxy.clone();
                let canvas = web_sys::window()
                    .unwrap()
                    .document()
                    .unwrap()
                    .get_element_by_id(canvas_id)
                    .unwrap()
                    .dyn_into::<web_sys::HtmlCanvasElement>()
                    .unwrap();
                let window = window.clone();
                move |_: web_sys::Event| {
                    let existing_canvas_size = winit::dpi::LogicalSize::new(
                        canvas.client_width() as u32,
                        canvas.client_height() as u32,
                    );

                    window.set_inner_size(existing_canvas_size);
                    window.request_redraw();
                    event_loop_proxy
                        .send_event(sixtyfps_corelib::eventloop::CustomEvent::WakeUpAndPoll)
                        .ok();
                }
            };

            let resize_closure =
                wasm_bindgen::closure::Closure::wrap(Box::new(resize_canvas) as Box<dyn FnMut(_)>);
            web_sys::window()
                .unwrap()
                .add_event_listener_with_callback("resize", resize_closure.as_ref().unchecked_ref())
                .unwrap();
            resize_closure.forget();

            {
                let default_size = window.inner_size().to_logical(window.scale_factor());
                let new_size = winit::dpi::LogicalSize::new(
                    if existing_canvas_size.width > 0 {
                        existing_canvas_size.width
                    } else {
                        default_size.width
                    },
                    if existing_canvas_size.height > 0 {
                        existing_canvas_size.height
                    } else {
                        default_size.height
                    },
                );
                if new_size != default_size {
                    window.set_inner_size(new_size);
                }
            }

            let renderer = femtovg::renderer::OpenGl::new_from_html_canvas(window.canvas());
            (window, renderer)
        };

        let canvas = femtovg::Canvas::new(renderer).unwrap();

        GLRenderer {
            canvas: Some(canvas),
            #[cfg(target_arch = "wasm32")]
            window,
            #[cfg(target_arch = "wasm32")]
            event_loop_proxy,
            #[cfg(not(target_arch = "wasm32"))]
            windowed_context: Some(unsafe { windowed_context.make_not_current().unwrap() }),
        }
    }
}

impl GraphicsBackend for GLRenderer {
    type ItemRenderer = GLItemRenderer;

    fn new_renderer(&mut self, width: u32, height: u32, clear_color: &Color) -> GLItemRenderer {
        #[cfg(not(target_arch = "wasm32"))]
        let current_windowed_context =
            unsafe { self.windowed_context.take().unwrap().make_current().unwrap() };

        let mut canvas = self.canvas.take().unwrap();

        {
            let dpi_factor = current_windowed_context.window().scale_factor();
            canvas.set_size(width, height, dpi_factor as f32);
        }

        canvas.clear_rect(0, 0, width, height, clear_color.into());

        GLItemRenderer {
            canvas,
            windowed_context: current_windowed_context,
            clip_rects: Default::default(),
        }
    }

    fn flush_renderer(&mut self, renderer: GLItemRenderer) {
        let mut canvas = renderer.canvas;
        canvas.flush();

        #[cfg(not(target_arch = "wasm32"))]
        {
            renderer.windowed_context.swap_buffers().unwrap();

            self.windowed_context =
                Some(unsafe { renderer.windowed_context.make_not_current().unwrap() });
        }

        self.canvas = Some(canvas);
    }
    fn window(&self) -> &winit::window::Window {
        #[cfg(not(target_arch = "wasm32"))]
        return self.windowed_context.as_ref().unwrap().window();
        #[cfg(target_arch = "wasm32")]
        return &self.window;
    }
}

pub struct GLItemRenderer {
    canvas: femtovg::Canvas<femtovg::renderer::OpenGl>,

    #[cfg(not(target_arch = "wasm32"))]
    windowed_context: glutin::WindowedContext<glutin::PossiblyCurrent>,

    clip_rects: SharedVector<Rect>,
}

impl GLItemRenderer {
    fn load_image(&mut self, source: Resource) -> Option<femtovg::ImageId> {
        match source {
            Resource::None => None,
            Resource::AbsoluteFilePath(path) => self
                .canvas
                .load_image_file(std::path::Path::new(&path.as_str()), femtovg::ImageFlags::empty())
                .ok(),
            Resource::EmbeddedData(data) => {
                self.canvas.load_image_mem(data.as_slice(), femtovg::ImageFlags::empty()).ok()
            }
            Resource::EmbeddedRgbaImage { width, height, data } => todo!(),
        }
    }
}

fn rect_to_path(r: Rect) -> femtovg::Path {
    let mut path = femtovg::Path::new();
    path.rect(r.min_x(), r.min_y(), r.width(), r.height());
    path
}

impl ItemRenderer for GLItemRenderer {
    fn draw_rectangle(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        rect: std::pin::Pin<&sixtyfps_corelib::items::Rectangle>,
    ) {
        // TODO: cache path in item to avoid re-tesselation
        let mut path = rect_to_path(rect.geometry());
        let paint = femtovg::Paint::color(
            sixtyfps_corelib::items::Rectangle::FIELD_OFFSETS.color.apply_pin(rect).get().into(),
        );
        self.canvas.save_with(|canvas| {
            canvas.translate(pos.x, pos.y);
            canvas.fill_path(&mut path, paint)
        })
    }

    fn draw_border_rectangle(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        rect: std::pin::Pin<&sixtyfps_corelib::items::BorderRectangle>,
    ) {
        // TODO: cache path in item to avoid re-tesselation
        let mut path = femtovg::Path::new();
        path.rounded_rect(
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS.x.apply_pin(rect).get(),
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS.y.apply_pin(rect).get(),
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS.width.apply_pin(rect).get(),
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS.height.apply_pin(rect).get(),
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS
                .border_radius
                .apply_pin(rect)
                .get(),
        );
        let fill_paint = femtovg::Paint::color(
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS
                .color
                .apply_pin(rect)
                .get()
                .into(),
        );
        let mut border_paint = femtovg::Paint::color(
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS
                .border_color
                .apply_pin(rect)
                .get()
                .into(),
        );
        border_paint.set_line_width(
            sixtyfps_corelib::items::BorderRectangle::FIELD_OFFSETS
                .border_width
                .apply_pin(rect)
                .get(),
        );
        self.canvas.save_with(|canvas| {
            canvas.translate(pos.x, pos.y);
            canvas.fill_path(&mut path, fill_paint)
        })
    }

    fn draw_image(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        image: std::pin::Pin<&sixtyfps_corelib::items::Image>,
    ) {
        // TODO: cache
        let image_id = self
            .load_image(sixtyfps_corelib::items::Image::FIELD_OFFSETS.source.apply_pin(image).get())
            .unwrap();

        let info = self.canvas.image_info(image_id).unwrap();

        let (image_width, image_height) = (info.width() as f32, info.height() as f32);
        let (source_width, source_height) = (image_width, image_height);
        let fill_paint =
            femtovg::Paint::image(image_id, 0., 0., source_width, source_height, 0.0, 1.0);

        let mut path = femtovg::Path::new();
        path.rect(0., 0., image_width, image_height);

        self.canvas.save_with(|canvas| {
            canvas.translate(pos.x, pos.y);

            let scaled_width =
                sixtyfps_corelib::items::Image::FIELD_OFFSETS.width.apply_pin(image).get();
            let scaled_height =
                sixtyfps_corelib::items::Image::FIELD_OFFSETS.height.apply_pin(image).get();
            if scaled_width > 0. && scaled_height > 0. {
                canvas.scale(scaled_width / image_width, scaled_height / image_height);
            }

            canvas.fill_path(&mut path, fill_paint);
        })
    }

    fn draw_clipped_image(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        clipped_image: std::pin::Pin<&sixtyfps_corelib::items::ClippedImage>,
    ) {
        // TODO: cache
        let image_id = self
            .load_image(
                sixtyfps_corelib::items::ClippedImage::FIELD_OFFSETS
                    .source
                    .apply_pin(clipped_image)
                    .get(),
            )
            .unwrap();

        let info = self.canvas.image_info(image_id).unwrap();

        let source_clip_rect = Rect::new(
            [
                sixtyfps_corelib::items::ClippedImage::FIELD_OFFSETS
                    .source_clip_x
                    .apply_pin(clipped_image)
                    .get() as _,
                sixtyfps_corelib::items::ClippedImage::FIELD_OFFSETS
                    .source_clip_y
                    .apply_pin(clipped_image)
                    .get() as _,
            ]
            .into(),
            [0., 0.].into(),
        );

        let (image_width, image_height) = (info.width() as f32, info.height() as f32);
        let (source_width, source_height) = if source_clip_rect.is_empty() {
            (image_width, image_height)
        } else {
            (source_clip_rect.width() as _, source_clip_rect.height() as _)
        };
        let fill_paint = femtovg::Paint::image(
            image_id,
            source_clip_rect.min_x(),
            source_clip_rect.min_y(),
            source_width,
            source_height,
            0.0,
            1.0,
        );

        let mut path = femtovg::Path::new();
        path.rect(pos.x, pos.y, image_width, image_height);

        self.canvas.save_with(|canvas| {
            let scaled_width = sixtyfps_corelib::items::ClippedImage::FIELD_OFFSETS
                .width
                .apply_pin(clipped_image)
                .get();
            let scaled_height = sixtyfps_corelib::items::ClippedImage::FIELD_OFFSETS
                .height
                .apply_pin(clipped_image)
                .get();
            if scaled_width > 0. && scaled_height > 0. {
                canvas.scale(scaled_width / image_width, scaled_height / image_height);
            }

            canvas.fill_path(&mut path, fill_paint);
        })
    }

    fn draw_text(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        rect: std::pin::Pin<&sixtyfps_corelib::items::Text>,
    ) {
        //todo!()
    }

    fn draw_text_input(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        rect: std::pin::Pin<&sixtyfps_corelib::items::TextInput>,
    ) {
        //todo!()
    }

    fn draw_path(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        path: std::pin::Pin<&sixtyfps_corelib::items::Path>,
    ) {
        //todo!()
    }

    fn combine_clip(
        &mut self,
        pos: sixtyfps_corelib::graphics::Point,
        clip: &std::pin::Pin<&sixtyfps_corelib::items::Clip>,
    ) {
        let clip_rect = clip.geometry().translate([pos.x, pos.y].into());
        self.canvas.intersect_scissor(
            clip_rect.min_x(),
            clip_rect.min_y(),
            clip_rect.width(),
            clip_rect.height(),
        );
        self.clip_rects.push(clip_rect);
    }

    fn clip_rects(&self) -> SharedVector<sixtyfps_corelib::graphics::Rect> {
        self.clip_rects.clone()
    }

    fn reset_clip(&mut self, rects: SharedVector<sixtyfps_corelib::graphics::Rect>) {
        self.clip_rects = rects;
        // ### Only do this if rects were really changed
        self.canvas.reset_scissor();
        for rect in self.clip_rects.as_slice() {
            self.canvas.intersect_scissor(rect.min_x(), rect.min_y(), rect.width(), rect.height())
        }
    }
}

pub fn create_gl_window() -> ComponentWindow {
    ComponentWindow::new(GraphicsWindow::new(|event_loop, window_builder| {
        GLRenderer::new(
            &event_loop.get_winit_event_loop(),
            window_builder,
            #[cfg(target_arch = "wasm32")]
            "canvas",
        )
    }))
}

#[cfg(target_arch = "wasm32")]
pub fn create_gl_window_with_canvas_id(canvas_id: String) -> ComponentWindow {
    ComponentWindow::new(GraphicsWindow::new(move |event_loop, window_builder| {
        GLRenderer::new(&event_loop.get_winit_event_loop(), window_builder, &canvas_id)
    }))
}

#[doc(hidden)]
#[cold]
pub fn use_modules() {
    sixtyfps_corelib::use_modules();
}

pub type NativeWidgets = ();
pub type NativeGlobals = ();
pub mod native_widgets {}
pub const HAS_NATIVE_STYLE: bool = false;
