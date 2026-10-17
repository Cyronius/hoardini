use super::{Kind, Pinned};

pub const RCLONE: Pinned = Pinned {
    id: "rclone",
    version: "1.75.1",
    url: "https://downloads.rclone.org/v1.75.1/rclone-v1.75.1-windows-amd64.zip",
    sha256: "200eb602c126d82aa38b51e0f6b9ae837473ff99b51278d3f6f837574c494d6e",
    size: 31_488_864,
    kind: Kind::Zip {
        extract: &["rclone-v1.75.1-windows-amd64/rclone.exe"],
    },
    license_url: "https://github.com/rclone/rclone/blob/v1.75.1/COPYING",
};

pub const LLAMA_SERVER: Pinned = Pinned {
    id: "llama.cpp",
    version: "b11503",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11503/llama-b11503-bin-win-cpu-x64.zip",
    sha256: "1f96923cab15cbcf07d3c1f8c59e75c24a14b703ff1d3da14ff866fddd97d775",
    size: 19_483_533,
    kind: Kind::Zip {
        extract: &[
            "llama-server.exe",
            "llama-server-impl.dll",
            "llama.dll",
            "llama-common.dll",
            "mtmd.dll",
            "ggml.dll",
            "ggml-base.dll",
            "ggml-rpc.dll",
            "ggml-cpu-x64.dll",
            "ggml-cpu-sse42.dll",
            "ggml-cpu-sandybridge.dll",
            "ggml-cpu-ivybridge.dll",
            "ggml-cpu-piledriver.dll",
            "ggml-cpu-haswell.dll",
            "ggml-cpu-alderlake.dll",
            "ggml-cpu-skylakex.dll",
            "ggml-cpu-cannonlake.dll",
            "ggml-cpu-cascadelake.dll",
            "ggml-cpu-cooperlake.dll",
            "ggml-cpu-icelake.dll",
            "ggml-cpu-sapphirerapids.dll",
            "ggml-cpu-zen4.dll",
            "libomp.dll",
            "LICENSE-LLVM-OpenMP",
        ],
    },
    license_url: "https://github.com/ggml-org/llama.cpp/blob/b11503/LICENSE",
};

pub const LLAMA_SERVER_VULKAN: Pinned = Pinned {
    id: "llama.cpp-vulkan",
    version: "b11503",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11503/llama-b11503-bin-win-vulkan-x64.zip",
    sha256: "aacab515e72c4c5bbfd8d2b54de5c08ff23848245727f5667aa2a854bc1617ab",
    size: 33_425_617,
    kind: Kind::Zip {
        extract: &[
            "llama-server.exe",
            "llama-server-impl.dll",
            "llama.dll",
            "llama-common.dll",
            "mtmd.dll",
            "ggml.dll",
            "ggml-base.dll",
            "ggml-rpc.dll",
            "ggml-vulkan.dll",
            "ggml-cpu-x64.dll",
            "ggml-cpu-sse42.dll",
            "ggml-cpu-sandybridge.dll",
            "ggml-cpu-ivybridge.dll",
            "ggml-cpu-piledriver.dll",
            "ggml-cpu-haswell.dll",
            "ggml-cpu-alderlake.dll",
            "ggml-cpu-skylakex.dll",
            "ggml-cpu-cannonlake.dll",
            "ggml-cpu-cascadelake.dll",
            "ggml-cpu-cooperlake.dll",
            "ggml-cpu-icelake.dll",
            "ggml-cpu-sapphirerapids.dll",
            "ggml-cpu-zen4.dll",
            "libomp.dll",
            "LICENSE-LLVM-OpenMP",
        ],
    },
    license_url: "https://github.com/ggml-org/llama.cpp/blob/b11503/LICENSE",
};

// Hugging Face LFS oid (= sha256) and size, read from the tree API at the pinned commit.
pub const MODELS: &[Pinned] = &[
    Pinned {
        id: "qwen3.5-9b",
        version: "q4_k_s",
        url: "https://huggingface.co/unsloth/Qwen3.5-9B-MTP-GGUF/resolve/9716a636ee4bddc3fed678220b7a33dd2a4160ae/Qwen3.5-9B-Q4_K_S.gguf",
        sha256: "8870a76668f73457fc109e1564d0ba9ed7b3fc23e552227ee062618831f9c9d2",
        size: 5_577_290_080,
        kind: Kind::File,
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
    },
    Pinned {
        id: "granite-4.1-3b",
        version: "q4_k_s",
        url: "https://huggingface.co/unsloth/granite-4.1-3b-GGUF/resolve/5b88826e4b80789548180f8faab39c5cf68772c9/granite-4.1-3b-Q4_K_S.gguf",
        sha256: "b054d1deb804080bd8e13f5e8876d9124149c611f77b4ffb9c01c8f9821e76a5",
        size: 1_998_372_160,
        kind: Kind::File,
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
    },
];

pub const PINS: &[&Pinned] = &[
    &RCLONE,
    &LLAMA_SERVER,
    &LLAMA_SERVER_VULKAN,
    &MODELS[0],
    &MODELS[1],
];

pub fn find(id: &str) -> Option<&'static Pinned> {
    PINS.iter().copied().find(|p| p.id == id)
}
