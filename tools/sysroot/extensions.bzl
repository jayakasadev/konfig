"""Module extension that exposes the linux/arm64 sysroot as a Bzlmod repo."""

load(":sysroot.bzl", "linux_arm64_sysroot")

def _sysroot_ext_impl(ctx):
    linux_arm64_sysroot(name = "linux_arm64_sysroot")

sysroot_ext = module_extension(
    implementation = _sysroot_ext_impl,
    doc = "Provides @linux_arm64_sysroot//:sysroot for linux/arm64 cross-compilation.",
)
