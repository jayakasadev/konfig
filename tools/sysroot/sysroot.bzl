"""Repository rule that extracts a Linux sysroot from a Docker image.

Requires Docker to be running on the host. The sysroot is extracted once
per arch and cached by Bazel — subsequent builds use the cached result.

Usage in MODULE.bazel:
    sysroot_ext = use_extension("//tools/sysroot:extensions.bzl", "sysroot_ext", dev_dependency = True)
    use_repo(sysroot_ext, "linux_arm64_sysroot", "linux_amd64_sysroot")
"""

# Per-arch settings:
#   docker_platform — `docker run --platform` value.
#   triple          — Debian/Ubuntu multiarch dir (e.g. `aarch64-linux-gnu`).
#   extra_paths     — extra absolute paths to include in the tar. amd64
#                     needs `/lib64` for the dynamic linker; arm64 doesn't
#                     have a `lib64` dir at all.
_ARCH_CONFIG = {
    "arm64": struct(
        docker_platform = "linux/arm64",
        triple = "aarch64-linux-gnu",
        extra_paths = "",
    ),
    "amd64": struct(
        docker_platform = "linux/amd64",
        triple = "x86_64-linux-gnu",
        # libc.so on amd64 GROUP-references /lib64/ld-linux-x86-64.so.2.
        extra_paths = "lib64",
    ),
}

def _linux_sysroot_impl(ctx):
    cfg = _ARCH_CONFIG.get(ctx.attr.arch)
    if not cfg:
        fail("Unknown arch '{}': expected one of {}".format(
            ctx.attr.arch,
            sorted(_ARCH_CONFIG.keys()),
        ))

    ctx.report_progress("Extracting linux/{} sysroot from Docker (runs once, then cached)".format(ctx.attr.arch))

    # Minimum sysroot contents for C++ cross-compilation:
    #   - usr/include          — headers (glibc, linux kernel)
    #   - usr/lib/<triple>     — libc, libstdc++, crt*.o
    #   - usr/lib/gcc/<triple> — gcc startup files
    #   - lib                  — dynamic linker + core libs (arm64)
    #   - lib64                — dynamic linker (amd64 only)
    script = """
set -euo pipefail

docker run --rm --platform {platform} ubuntu:22.04 bash -c "
  apt-get update -q >/dev/null 2>&1 &&
  apt-get install -y -q --no-install-recommends \\
    libc6-dev libstdc++-12-dev >/dev/null 2>&1 &&
  tar --dereference -c \\
    usr/include \\
    usr/lib/{triple} \\
    usr/lib/gcc/{triple} \\
    lib \\
    {extra_paths}
" | tar -x --no-same-owner --no-same-permissions

echo "Sysroot extraction complete"
""".format(
        platform = cfg.docker_platform,
        triple = cfg.triple,
        extra_paths = cfg.extra_paths,
    )

    result = ctx.execute(
        ["bash", "-c", script],
        timeout = 600,
        quiet = False,
    )

    if result.return_code != 0:
        fail("Failed to extract linux/{} sysroot from Docker: {}".format(
            ctx.attr.arch,
            result.stderr,
        ))

    ctx.file("BUILD.bazel", """
filegroup(
    name = "sysroot",
    srcs = glob(
        ["usr/**", "lib/**", "lib64/**"],
        allow_empty = True,
    ),
    visibility = ["//visibility:public"],
)
""")

linux_sysroot = repository_rule(
    implementation = _linux_sysroot_impl,
    local = True,
    configure = True,
    attrs = {
        "arch": attr.string(
            mandatory = True,
            doc = "Target architecture: arm64 or amd64.",
        ),
    },
    doc = "Extracts a minimal Linux glibc sysroot from Ubuntu 22.04 via Docker.",
)
