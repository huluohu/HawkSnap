//! 贴图窗口冒烟验证（授权机手动跑；CI 不跑）。main 在进程主线程——AppKit 窗口
//! 创建的合法位置（libtest 的测试线程不是主线程，故不能走 #[test] 路径）。
//!
//! 运行：`cargo run -p hawksnap-platform-mac --example pin_smoke`
//! 观测：左上角附近出现一枚 100×100 红色贴图（半尺寸起步）→ 放大到原始尺寸 →
//! 半透明 → 2 秒后消失。
//! 通过判据（非仅「无 panic」）：自转 RunLoop 让 AppKit 处理完 ordering 后，
//! 窗口 isVisible == true——**未上屏直接 panic**（E2E 缺陷回归闸：曾因「只断言
//! 不 panic」漏检过窗口可见性问题）。
//!
//! 非 macOS 目标：examples 随 `cargo test` 一同构建，必须可编译——打印一行说明退出。

fn main() {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Arc;

        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};

        use hawksnap_platform_mac::pin::PinWindow;

        let mtm = MainThreadMarker::new().expect("pin_smoke 必须在进程主线程运行");
        let _app = NSApplication::sharedApplication(mtm);

        let mut rgba = vec![0u8; 100 * 100 * 4];
        for px in rgba.chunks_exact_mut(4) {
            px[0] = 255; // R
            px[3] = 255; // A
        }
        let pin = PinWindow::create(Arc::new(rgba), 100, 100, (120.0, 120.0), 2.0, 0)
            .expect("创建贴图窗口");
        println!("scale after create = {}", pin.scale()); // 0.5（半尺寸）

        // 自转一下 RunLoop：让 AppKit 消化 orderFrontRegardless（isVisible 通常同步置位，
        // 这里转 0.5s 是给窗口服务器上报留余量，属一次性成本，非常驻）。
        let run_loop = NSRunLoop::mainRunLoop();
        let limit = NSDate::dateWithTimeIntervalSinceNow(0.5);
        // SAFETY: NSDefaultRunLoopMode 是 AppKit 固有的全局常量字符串（extern static），
        // 地址恒定有效；仅作 runMode 参数读取。
        let default_mode = unsafe { NSDefaultRunLoopMode };
        run_loop.runMode_beforeDate(&default_mode, &limit);

        // 硬断言：窗口必须真实上屏（0.3.2 无 isOnScreen 绑定，isVisible = 已 order 上屏）。
        assert!(
            pin.is_visible(),
            "贴图窗口未上屏：create 后 isVisible=false（orderFrontRegardless 未生效）"
        );
        println!("visible = true");

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

    #[cfg(not(target_os = "macos"))]
    {
        println!("pin_smoke 仅限 macOS（贴图窗口是 AppKit 原生实现）；本平台为 no-op。");
    }
}
