# dgi

Minimal native Windows desktop application built with Rust and `native-windows-gui` (MIT).

## Architecture
- **Framework**: `native-windows-gui` v1.0.13 (pure native Win32 controls, no WebView, no .NET, no Slint).
- **Subsystem**: `#![windows_subsystem = "windows"]` (zero console window flash).
- **Window**: Resizable native Win32 window titled `dgi` (~1000x700), standard minimize/maximize/close controls.
- **DPI**: Per-monitor V2 DPI awareness (`SetProcessDpiAwarenessContext(-4)`).
- **Shutdown**: Clean thread dispatch termination on `WM_CLOSE` / `OnWindowClose`.

## Prerequisites
- Rust 1.95+ toolchain with `x86_64-pc-windows-msvc`
- MSVC C++ Build Tools linker (`link.exe`)

## Build & Run

### Debug Build & Run
```powershell
cargo run
```

### Release Build
```powershell
cargo build --release
```

The release executable is generated at:
```powershell
.\target\release\dgi.exe
```

### Direct Execution
```powershell
.\target\release\dgi.exe
```
