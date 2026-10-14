fn main() {
    // VCRUNTIME140.dll - статически, см. Cargo.toml
    #[cfg(windows)]
    static_vcruntime::metabuild();
}
