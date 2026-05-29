"""Module extension that exposes Linux sysroots as Bzlmod repos."""

load(":sysroot.bzl", "linux_sysroot")

def _sysroot_ext_impl(ctx):
    linux_sysroot(name = "linux_arm64_sysroot", arch = "arm64")
    linux_sysroot(name = "linux_amd64_sysroot", arch = "amd64")

sysroot_ext = module_extension(
    implementation = _sysroot_ext_impl,
    doc = "Provides @linux_arm64_sysroot//:sysroot and @linux_amd64_sysroot//:sysroot for Linux cross-compilation.",
)
