//! Display-backed regression check for the CEF renderer. No Mapbox token needed.
//! Run with: cargo run --example map_render_probe
use iced_webview::{Cef, Engine, PageType};
use std::time::{Duration, Instant};

fn main() {
    if iced_webview::cef_subprocess_check() {
        return;
    }
    let mut engine = Cef::try_new().expect("CEF initialization");
    engine.set_scale_factor(2.0);
    let html = r#"<style>body{margin:0;background:black}</style><canvas id="c"></canvas><script>
const gl=document.getElementById('c').getContext('webgl2');
const ext=gl?.getExtension('WEBGL_debug_renderer_info');
let click=null; document.addEventListener('click',event=>{click=[event.clientX,event.clientY];});
setInterval(()=>{document.title='PROBE:'+JSON.stringify({dpr:devicePixelRatio,width:innerWidth,click,gpu:ext?gl.getParameter(ext.UNMASKED_RENDERER_WEBGL):'unknown'});},50);
</script>"#;
    let view = engine.new_view(iced::Size::new(640, 480), Some(PageType::Html(html.into())));
    for (scale, width, height) in [(2.0, 1280, 960), (1.25, 800, 600), (1.0, 640, 480)] {
        engine.set_scale_factor(scale);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            engine.update();
            let title = engine.get_title(view);
            let image = engine.get_view(view);
            if let Some(json) = title.strip_prefix("PROBE:") {
                let info: serde_json::Value = serde_json::from_str(json).unwrap();
                let painted = match image.as_handle() {
                    iced::widget::image::Handle::Rgba { pixels, .. } => pixels[0] == 0,
                    _ => false,
                };
                if (info["dpr"].as_f64().unwrap() - scale as f64).abs() < 0.001
                    && info["width"] == 640
                    && image.image_width() == width
                    && image.image_height() == height
                    && painted
                {
                    // Verify a logical map click survives the physical surface/page zoom transform.
                    let point = iced::Point::new(100.0, 150.0);
                    engine.handle_mouse_event(
                        view,
                        point,
                        iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left),
                    );
                    engine.handle_mouse_event(
                        view,
                        point,
                        iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left),
                    );
                    for _ in 0..20 {
                        engine.update();
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let title = engine.get_title(view);
                    let info: serde_json::Value =
                        serde_json::from_str(title.strip_prefix("PROBE:").unwrap()).unwrap();
                    assert_eq!(info["click"], serde_json::json!([100, 150]));
                    println!(
                        "scale={scale}, frame={width}x{height}, click aligned, GPU={}",
                        info["gpu"]
                    );
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "scale {scale}: {title}, frame={}x{}",
                image.image_width(),
                image.image_height()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    engine.remove_view(view);
    for _ in 0..20 {
        engine.update();
        std::thread::sleep(Duration::from_millis(10));
    }
}
