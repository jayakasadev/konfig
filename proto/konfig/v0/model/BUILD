load("@rules_proto//proto:defs.bzl", "proto_library")
load("@rules_proto_grpc_go//:defs.bzl", "go_grpc_library")

filegroup(
    name = "srcs",
    srcs = glob(["*.proto"]),
    visibility = ["//visibility:public"],
)

proto_library(
    name = "proto",
    srcs = glob(["*.proto"]),
    visibility = ["//visibility:public"],
    deps = [
        "//proto/konfig/v0/common:proto",
        "@protobuf//:timestamp_proto",
    ],
)

go_grpc_library(
    name = "go_default_library",
    importpath = "capitalintent.com/konfig/protos/v0/model",
    protos = [":proto"],
    visibility = ["//go/konfig:__subpackages__"],
    deps = [
        "//proto/konfig/v0/common:go_default_library",
    ],
)
