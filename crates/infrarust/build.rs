fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=INFRARUST_OFFICIAL_BUILD");
    println!("cargo::rerun-if-env-changed=INFRARUST_DOCKER_IMAGE");

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo::rustc-env=INFRARUST_TARGET={target}");

    let official = std::env::var("INFRARUST_OFFICIAL_BUILD").is_ok_and(|value| value == "1");
    println!("cargo::rustc-env=INFRARUST_OFFICIAL={}", u8::from(official));

    let docker = std::env::var("INFRARUST_DOCKER_IMAGE").is_ok_and(|value| value == "1");
    println!("cargo::rustc-env=INFRARUST_DOCKER={}", u8::from(docker));
}
