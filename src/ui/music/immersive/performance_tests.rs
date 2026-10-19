use super::*;

fn percentile(samples: &mut [f64], fraction: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * fraction).round() as usize]
}

#[test]
#[ignore = "Synthetic immersive CPU profile; run alone, no display or audio required"]
fn profile_immersive_cairo_drawing_and_spectrum_analysis() {
    let mut analyzer = crate::playback::spectrum::SpectrumAnalyzer::new();
    let spectrum = [0.5; crate::playback::SPECTRUM_BANDS];
    for (width, height) in [(1280, 720), (1920, 1080), (3840, 2160)] {
        for style in [
            VisualizerStyle::Ribbons,
            VisualizerStyle::Tunnel,
            VisualizerStyle::Cosmic,
        ] {
            let surface =
                gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
                    .unwrap();
            let context = gtk::cairo::Context::new(&surface).unwrap();
            let mut samples = Vec::new();
            for frame in 0..20 {
                context.set_operator(gtk::cairo::Operator::Clear);
                context.paint().unwrap();
                context.set_operator(gtk::cairo::Operator::Over);
                let started = std::time::Instant::now();
                draw_generative_visualizer(
                    &context,
                    width as f64,
                    height as f64,
                    frame as f64 / 60.0,
                    style,
                    &spectrum,
                    (0.5, 0.7, 0.9),
                    (0.8, 0.6, 0.4),
                );
                surface.flush();
                if frame >= 3 {
                    samples.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            println!(
                "cairo {style:?} {width}x{height} median_ms={:.3} p95_ms={:.3}",
                percentile(&mut samples, 0.5),
                percentile(&mut samples, 0.95)
            );
        }
    }
    let mut pcm = vec![0u8; crate::playback::spectrum::ANALYSIS_SAMPLES * 8];
    for (index, frame) in pcm.chunks_exact_mut(8).enumerate() {
        let sample = ((index as f32 * 0.1).sin() * 0.4).to_le_bytes();
        frame[..4].copy_from_slice(&sample);
        frame[4..].copy_from_slice(&sample);
    }
    let mut samples = Vec::new();
    for _ in 0..60 {
        analyzer.push_pcm(&pcm);
        let started = std::time::Instant::now();
        std::hint::black_box(analyzer.analyze());
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "spectrum median_ms={:.3} p95_ms={:.3}",
        percentile(&mut samples, 0.5),
        percentile(&mut samples, 0.95)
    );
}

struct RendererGuard(gtk::gsk::Renderer);

fn renderer_device_counters() -> Vec<String> {
    let mut counters = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc/self/fdinfo") {
        for entry in entries.flatten() {
            if let Ok(info) = std::fs::read_to_string(entry.path()) {
                counters.extend(
                    info.lines()
                        .filter(|line| {
                            line.starts_with("drm-driver:") || line.starts_with("drm-engine-")
                        })
                        .map(str::to_owned),
                );
            }
        }
    }
    counters.sort();
    counters.dedup();
    counters
}

impl Drop for RendererGuard {
    fn drop(&mut self) {
        if self.0.is_realized() {
            self.0.unrealize();
        }
    }
}

#[test]
#[ignore = "Isolated GTK offscreen renderer profile; never presents a window or opens media"]
fn profile_immersive_offscreen_renderer_cairo_and_native_nodes() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    gtk::prelude::WidgetExt::realize(&window);
    assert!(!window.is_mapped());
    assert!(!window.is_visible());
    let renderer = RendererGuard(if std::env::var_os("HERMES_PROFILE_SOFTWARE").is_some() {
        let renderer = gtk::gsk::CairoRenderer::new();
        renderer.realize(window.surface().as_ref()).unwrap();
        renderer.upcast()
    } else {
        gtk::gsk::Renderer::for_surface(&window.surface().unwrap()).unwrap()
    });
    println!(
        "renderer={} display={}",
        renderer.0.type_().name(),
        gtk::prelude::WidgetExt::display(&window).type_().name()
    );
    println!("device_before={:?}", renderer_device_counters());
    let spectrum = [0.5; crate::playback::SPECTRUM_BANDS];
    for (width, height) in [(1280, 720), (1920, 1080), (3840, 2160)] {
        for style in [
            VisualizerStyle::Ribbons,
            VisualizerStyle::Tunnel,
            VisualizerStyle::Cosmic,
        ] {
            for native in [false, true] {
                let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
                let mut recording = Vec::new();
                let mut completed = Vec::new();
                for frame in 0..15 {
                    let started = std::time::Instant::now();
                    let snapshot = gtk::Snapshot::new();
                    if native {
                        canvas::snapshot_frame(
                            &snapshot,
                            width as f64,
                            height as f64,
                            style,
                            frame as f64 / 60.0,
                            spectrum,
                            ((0.5, 0.7, 0.9), (0.8, 0.6, 0.4)),
                        );
                    } else {
                        let context = snapshot.append_cairo(&bounds);
                        draw_generative_visualizer(
                            &context,
                            width as f64,
                            height as f64,
                            frame as f64 / 60.0,
                            style,
                            &spectrum,
                            (0.5, 0.7, 0.9),
                            (0.8, 0.6, 0.4),
                        );
                        drop(context);
                    }
                    let node = snapshot.to_node().unwrap();
                    let record_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let texture = renderer.0.render_texture(&node, Some(&bounds));
                    let downloader = gdk::TextureDownloader::new(&texture);
                    std::hint::black_box(downloader.download_bytes());
                    if frame >= 3 {
                        recording.push(record_ms);
                        completed.push(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    if frame == 14 && width == 1280 {
                        if let Ok(directory) = std::env::var("HERMES_RENDER_SHOTS") {
                            texture
                                .save_to_png(std::path::Path::new(&directory).join(format!(
                                    "{style:?}-{}.png",
                                    if native { "native" } else { "cairo" }
                                )))
                                .unwrap();
                        }
                    }
                }
                println!(
                    "{} {style:?} {width}x{height} record_median_ms={:.3} completed_median_ms={:.3} completed_p95_ms={:.3}",
                    if native { "gsk-native" } else { "cairo-node" },
                    percentile(&mut recording, 0.5),
                    percentile(&mut completed, 0.5),
                    percentile(&mut completed, 0.95)
                );
            }
        }
    }
    assert!(!window.is_mapped());
    println!("device_after={:?}", renderer_device_counters());
    drop(renderer);
    window.destroy();
}
