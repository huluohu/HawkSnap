//! 贴图窗口冒烟验证（授权机手动跑；CI 不跑）。main 在进程主线程——AppKit 窗口
//! 创建的合法位置（libtest 的测试线程不是主线程，故不能走 #[test] 路径）。
//!
//! 运行：`cargo run -p hawksnap-platform-mac --example pin_smoke`
//! 观测：左上角附近出现一枚 100×100 红色贴图（半尺寸起步）→ 放大到原始尺寸 →
//! 半透明 → 2 秒后消失；全程无 panic 即通过。
use std::sync::Arc;

use hawksnap_platform_mac::pin::PinWindow;

fn main() {
    let mut rgba = vec![0u8; 100 * 100 * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[0] = 255; // R
        px[3] = 255; // A
    }
    let pin = PinWindow::create(Arc::new(rgba), 100, 100, (120.0, 120.0), 2.0, 0)
        .expect("创建贴图窗口");
    println!("scale after create = {}", pin.scale()); // 0.5（半尺寸）
    pin.zoom_to(1.0);
    println!("scale after zoom = {}", pin.scale()); // 1.0（原始尺寸）
    pin.set_opacity(0.5);
    println!("opacity set to 0.5");
    std::thread::sleep(std::time::Duration::from_millis(2000));
    pin.close();
    pin.close(); // 幂等
    drop(pin);
    println!("pin smoke OK");
}
