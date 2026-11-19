fn main() {
    println!("cargo:rerun-if-changed=src");
    if std::env::var_os("ROUNDHOUSE_BUILDING_CONTRACT").is_some() {
        return;
    }
    roundhouse_build::prepare().expect("roundhouse contract generation failed");
}
