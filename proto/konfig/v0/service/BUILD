load("@rules_proto//proto:defs.bzl", "proto_descriptor_set", "proto_library")
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
        "@googleapis//google/api:annotations_proto",
        "@protobuf//:empty_proto",
        "@protobuf//:timestamp_proto",
    ],
)

proto_descriptor_set(
    name = "descriptor_set",
    deps = [
        ":proto",
        "@googleapis//google/api:annotations_proto",
    ],
)

go_grpc_library(
    name = "go_default_library",
    importpath = "capitalintent.com/konfig/protos/v0/service",
    #    options = {
    #        "@rules_proto_grpc//grpc-gateway:grpc_gateway_plugin": ["allow_delete_body=true"],
    #    },
    protos = [":proto"],
    visibility = ["//visibility:public"],
    deps = [
        "//proto/konfig/v0/common:go_default_library",
    ],
)
