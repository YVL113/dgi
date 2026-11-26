// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let [mode, input, output] = args.as_slice() {
        let result = if mode == "--decode-animation" || mode == "--encode-animation" {
            dgi_lib::decode_animation_file(
                input.as_ref(),
                output.as_ref(),
                mode == "--encode-animation",
            )
        } else if mode == "--preview-model" {
            dgi_lib::decode_model_file(input.as_ref(), output.as_ref())
        } else {
            return dgi_lib::run();
        };
        std::process::exit(i32::from(result.is_err()));
    }
    dgi_lib::run()
}
