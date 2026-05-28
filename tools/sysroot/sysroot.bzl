"""Repository rule that extracts a linux/arm64 sysroot from a Docker image.

Requires Docker to be running on the host. The sysroot is extracted once and
cached by Bazel — subsequent builds use the cached result.

Usage in MODULE.bazel:
    sysroot_ext = use_extension("//tools/sysroot:extensions.bzl", "sysroot_ext", dev_dependency = True)
    use_repo(sysroot_ext, "linux_arm64_sysroot")
"""

def _linux_arm64_sysroot_impl(ctx):
    ctx.report_progress("Extracting linux/arm64 sysroot from Docker (runs once, then cached)")

    # Minimum sysroot contents for C++ cross-compilation:
    #   - usr/include          — headers (glibc, linux kernel)
    #   - usr/lib              — static libraries + pkg-config
    #   - lib/aarch64-linux-gnu — dynamic linker + core libs (libc.so, libpthread.so, etc.)
    #   - usr/lib/aarch64-linux-gnu — libc, libstdc++, crt*.o
    result = ctx.execute(
        [
            "bash",
            "-c",
            """
set -euo pipefail

PLATFORM="linux/arm64"

# Run a linux/arm64 container, install C/C++ headers, then tar the sysroot
# paths to stdout and extract here. The base Ubuntu image has no dev headers —
# libc6-dev provides assert.h, stdio.h, etc.
docker run --rm --platform "$PLATFORM" ubuntu:22.04 bash -c "
  apt-get update -q >/dev/null 2>&1 &&
  apt-get install -y -q --no-install-recommends \
    libc6-dev libstdc++-12-dev >/dev/null 2>&1 &&
  tar --dereference -c \
    usr/include \
    usr/lib/aarch64-linux-gnu \
    usr/lib/gcc/aarch64-linux-gnu \
    lib
" | tar -x --no-same-owner --no-same-permissions

echo "Sysroot extraction complete"
""",
        ],
        timeout = 600,
        quiet = False,
    )

    if result.return_code != 0:
        fail("Failed to extract linux/arm64 sysroot from Docker: " + result.stderr)

    ctx.file("BUILD.bazel", """
filegroup(
    name = "sysroot",
    srcs = glob(
        ["usr/**", "lib/**"],
        allow_empty = False,
    ),
    visibility = ["//visibility:public"],
)
""")

linux_arm64_sysroot = repository_rule(
    implementation = _linux_arm64_sysroot_impl,
    local = True,
    configure = True,
    doc = "Extracts a minimal linux/arm64 glibc sysroot from Ubuntu 22.04 via Docker.",
)
