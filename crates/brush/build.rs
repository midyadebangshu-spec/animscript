//! Compiles the vendored libmypaint (`third_party/libmypaint`): the brush engine and the fixed tiled surface.
fn main() {
    let dir = std::path::Path::new("../../third_party/libmypaint");
    let files = [
        "brushmodes.c",
        "fifo.c",
        "helpers.c",
        "mypaint-brush-settings.c",
        "mypaint-brush.c",
        "mypaint-fixed-tiled-surface.c",
        "mypaint-mapping.c",
        "mypaint-matrix.c",
        "mypaint-rectangle.c",
        "mypaint-surface.c",
        "mypaint-symmetry.c",
        "mypaint-tiled-surface.c",
        "operationqueue.c",
        "rng-double.c",
        "tilemap.c",
    ];
    let mut b = cc::Build::new();
    b.include(dir)
        .define("MYPAINT_USE_JSON", "0")
        .std("gnu11")
        .opt_level(2)
        .warnings(false)
        // no fused multiply-add and no fast-math: the same arithmetic on every machine
        .flag_if_supported("-ffp-contract=off")
        .flag_if_supported("-fno-fast-math");
    for f in files {
        b.file(dir.join(f));
        println!("cargo:rerun-if-changed={}", dir.join(f).display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    b.compile("mypaint");
    if !cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=m");
    }
}
