// Copyright 2016 Joe Wilm, The Alacritty Project Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The display subsystem including window management, font rasterization, and
//! GPU drawing.
use std::sync::mpsc;

use parking_lot::{MutexGuard};

use font;
use Rgb;
use ansi::Color;
use cli;
use config::Config;
use meter::Meter;
use renderer::{GlyphCache, QuadRenderer};
use term::{Term, SizeInfo};

use window::{self, Size, Pixels, Window, SetInnerSize};

/// The display wraps a window, font rasterizer, and GPU renderer
pub struct Display {
    window: Window,
    renderer: QuadRenderer,
    glyph_cache: GlyphCache,
    render_timer: bool,
    rx: mpsc::Receiver<(u32, u32)>,
    tx: mpsc::Sender<(u32, u32)>,
    meter: Meter,
    size_info: SizeInfo,
}

/// Can wakeup the render loop from other threads
pub struct Notifier(window::Proxy);

/// Types that are interested in when the display is resized
pub trait OnResize {
    fn on_resize(&mut self, size: &SizeInfo);
}

impl Notifier {
    pub fn notify(&self) {
        self.0.wakeup_event_loop();
    }
}

impl Display {
    pub fn notifier(&self) -> Notifier {
        Notifier(self.window.create_window_proxy())
    }

    pub fn update_config(&mut self, config: &Config) {
        self.renderer.update_config(config);
        self.render_timer = config.render_timer();
    }

    /// Get size info about the display
    pub fn size(&self) -> &SizeInfo {
        &self.size_info
    }

    pub fn new(
        config: &Config,
        options: &cli::Options,
    ) -> Result<Display, window::Error> {
        // Extract some properties from config
        let font = config.font();
        let dpi = config.dpi();
        let render_timer = config.render_timer();

        // Create the window where Alacritty will be displayed
        let mut window = match Window::new() {
            Ok(window) => window,
            Err(err) => die!("{}", err)
        };

        // get window properties for initializing the other subsytems
        let size = window.inner_size_pixels().unwrap();
        let dpr = window.hidpi_factor();

        println!("device_pixel_ratio: {}", dpr);

        let rasterizer = font::Rasterizer::new(dpi.x(), dpi.y(), dpr);

        // Create renderer
        let mut renderer = QuadRenderer::new(&config, size);

        // Initialize glyph cache
        let glyph_cache = {
            println!("Initializing glyph cache");
            let init_start = ::std::time::Instant::now();

            let cache = renderer.with_loader(|mut api| {
                GlyphCache::new(rasterizer, &config, &mut api)
            });

            let stop = init_start.elapsed();
            let stop_f = stop.as_secs() as f64 + stop.subsec_nanos() as f64 / 1_000_000_000f64;
            println!("Finished initializing glyph cache in {}", stop_f);

            cache
        };

        // Need font metrics to resize the window properly. This suggests to me the
        // font metrics should be computed before creating the window in the first
        // place so that a resize is not needed.
        let metrics = glyph_cache.font_metrics();
        let cell_width = (metrics.average_advance + font.offset().x() as f64) as u32;
        let cell_height = (metrics.line_height + font.offset().y() as f64) as u32;

        // Resize window to specified dimensions
        let width = cell_width * options.columns_u32() + 4;
        let height = cell_height * options.lines_u32() + 4;
        let size = Size { width: Pixels(width), height: Pixels(height) };
        println!("set_inner_size: {}", size);

        window.set_inner_size(size);
        renderer.resize(*size.width as _, *size.height as _);
        println!("Cell Size: ({} x {})", cell_width, cell_height);

        let size_info = SizeInfo {
            width: *size.width as f32,
            height: *size.height as f32,
            cell_width: cell_width as f32,
            cell_height: cell_height as f32
        };

        // Channel for resize events
        //
        // macOS has a callback for getting resize events, the channel is used
        // to queue resize events until the next draw call. Unfortunately, it
        // seems that the event loop is blocked until the window is done
        // resizing. If any drawing were to happen during a resize, it would
        // need to be in the callback.
        let (tx, rx) = mpsc::channel();

        let mut display = Display {
            window: window,
            renderer: renderer,
            glyph_cache: glyph_cache,
            render_timer: render_timer,
            tx: tx,
            rx: rx,
            meter: Meter::new(),
            size_info: size_info,
        };

        let resize_tx = display.resize_channel();
        let proxy = display.window.create_window_proxy();
        display.window.set_resize_callback(move |width, height| {
            let _ = resize_tx.send((width, height));
            proxy.wakeup_event_loop();
        });

        Ok(display)
    }

    #[inline]
    pub fn resize_channel(&self) -> mpsc::Sender<(u32, u32)> {
        self.tx.clone()
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Process pending resize events
    pub fn handle_resize(&mut self, terminal: &mut MutexGuard<Term>, items: &mut [&mut OnResize]) {
        // Resize events new_size and are handled outside the poll_events
        // iterator. This has the effect of coalescing multiple resize
        // events into one.
        let mut new_size = None;

        // Take most recent resize event, if any
        while let Ok(sz) = self.rx.try_recv() {
            new_size = Some(sz);
        }

        // Receive any resize events; only call gl::Viewport on last
        // available
        if let Some((w, h)) = new_size.take() {
            terminal.resize(w as f32, h as f32);
            let size = terminal.size_info();

            for mut item in items {
                item.on_resize(size)
            }

            self.renderer.resize(w as i32, h as i32);
        }

    }

    /// Draw the screen
    ///
    /// A reference to Term whose state is being drawn must be provided.
    ///
    /// This call may block if vsync is enabled
    pub fn draw(&mut self, mut terminal: MutexGuard<Term>, config: &Config) {
        // This is a hack since sometimes we get stuck waiting for events
        // in the main loop otherwise.
        //
        // TODO figure out why this is necessary
        self.window.clear_wakeup_flag();

        // Clear dirty flag
        terminal.dirty = false;

        {
            let glyph_cache = &mut self.glyph_cache;
            // Draw grid
            {
                let _sampler = self.meter.sampler();

                let size_info = terminal.size_info().clone();
                self.renderer.with_api(config, &size_info, |mut api| {
                    api.clear();

                    // Draw the grid
                    api.render_cells(terminal.renderable_cells(), glyph_cache);
                });
            }

            // Draw render timer
            if self.render_timer {
                let timing = format!("{:.3} usec", self.meter.average());
                let color = Color::Spec(Rgb { r: 0xd5, g: 0x4e, b: 0x53 });
                self.renderer.with_api(config, terminal.size_info(), |mut api| {
                    api.render_string(&timing[..], glyph_cache, &color);
                });
            }
        }

        // Unlock the terminal mutex
        drop(terminal);
        self.window.swap_buffers().unwrap();
    }
}