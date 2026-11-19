fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-env-changed=ROUNDHOUSE_AUTO_CONTRACT");
    println!("cargo:rerun-if-env-changed=CI");
    roundhouse_build::prepare().expect("roundhouse contract generation failed");
}
