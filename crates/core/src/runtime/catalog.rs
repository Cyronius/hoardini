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

// Hugging Face LFS oid (= sha256) and size, read from the tree API at the pinned commit.
pub const MODELS: &[Pinned] = &[
    Pinned {
        id: "lfm2.5-1.2b-instruct",
        version: "q4_k_m",
        url: "https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-GGUF/resolve/8ed288026e23958ad9dfa92d53ed773a8eee7125/LFM2.5-1.2B-Instruct-Q4_K_M.gguf",
        sha256: "b1b3de114215d9507409a662a501a631095a479a419584e8a2ded6304b19b4f5",
        size: 730_895_168,
        kind: Kind::File,
        license_url: "https://huggingface.co/LiquidAI/LFM2.5-1.2B-Instruct-GGUF/blob/8ed288026e23958ad9dfa92d53ed773a8eee7125/LICENSE",
    },
    Pinned {
        id: "lfm2.5-2.6b",
        version: "q4_k_m",
        url: "https://huggingface.co/LiquidAI/LFM2.5-2.6B-GGUF/resolve/e7caca5d835a3901a8e0d63e94009429bafafdfc/LFM2.5-2.6B-Q4_K_M.gguf",
        sha256: "02a8b7e17487d326e46d68ce0ba24211e1b80a14c4cd0597fa73c1cd697f52ed",
        size: 1_674_455_040,
        kind: Kind::File,
        license_url: "https://huggingface.co/LiquidAI/LFM2.5-2.6B-GGUF/blob/e7caca5d835a3901a8e0d63e94009429bafafdfc/LICENSE",
    },
    Pinned {
        id: "qwen3-4b-instruct-2507",
        version: "q4_k_m",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        size: 2_497_281_120,
        kind: Kind::File,
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
    },
    Pinned {
        id: "granite-4.0-micro",
        version: "q4_k_m",
        url: "https://huggingface.co/ibm-granite/granite-4.0-micro-GGUF/resolve/ec48475f0c811d812fbfb61975717a9c36eeb652/granite-4.0-micro-Q4_K_M.gguf",
        sha256: "97c417dcc0534b0737c74016fb2af083cb17c3b51eaac621192d23961b7024eb",
        size: 2_099_502_528,
        kind: Kind::File,
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
    },
    Pinned {
        id: "minicpm5-2b",
        version: "q4_k_m",
        url: "https://huggingface.co/openbmb/MiniCPM5-2B-GGUF/resolve/2079a22f3beaa4e306449978533478fe0522f4b3/MiniCPM5-2B-Q4_K_M.gguf",
        sha256: "ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd",
        size: 1_561_318_368,
        kind: Kind::File,
        license_url: "https://www.apache.org/licenses/LICENSE-2.0",
    },
];

pub const PINS: &[&Pinned] = &[
    &RCLONE,
    &LLAMA_SERVER,
    &MODELS[0],
    &MODELS[1],
    &MODELS[2],
    &MODELS[3],
    &MODELS[4],
];

pub fn find(id: &str) -> Option<&'static Pinned> {
    PINS.iter().copied().find(|p| p.id == id)
}
