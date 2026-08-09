use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use super::metadata::{Metadata, parse_metadata};

/// Host hooks which the concrete worker installs for every test process.
///
/// Requirement discovery remains conservative and independent of execution;
/// the coordinator subtracts this typed capability set only after the worker
/// implementation has actually published the corresponding hook.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct HostCapabilities {
    pub agent: bool,
    pub can_block_false: bool,
    pub create_realm: bool,
    pub detach_array_buffer: bool,
    pub eval_script: bool,
    pub gc: bool,
    pub global: bool,
    pub is_html_dda: bool,
}

impl HostCapabilities {
    pub(super) fn retain_missing(self, capabilities: &mut Vec<String>) {
        capabilities.retain(|capability| match capability.as_str() {
            "agent" => !self.agent,
            "can-block:false" => !self.can_block_false,
            "create-realm" => !self.create_realm,
            "detach-array-buffer" => !self.detach_array_buffer,
            "eval-script" => !self.eval_script,
            "gc" => !self.gc,
            "global" => !self.global,
            "is-html-dda" => !self.is_html_dda,
            _ => true,
        });
    }
}

#[derive(Clone, Copy)]
struct NegativeMetadataContract {
    phase: &'static str,
    error_type: &'static str,
}

#[derive(Clone, Copy)]
struct ModuleMetadataContract {
    includes: &'static [&'static str],
    flags: &'static [&'static str],
    features: &'static [&'static str],
    negative: Option<NegativeMetadataContract>,
}

struct DependencyFreeModuleAdmission {
    path: &'static str,
    source_sha256: &'static str,
    metadata: ModuleMetadataContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactModuleTest {
    DependencyFree,
    FixtureGraph,
}

#[derive(Clone, Copy)]
struct ModuleRequestAdmission {
    specifier: &'static str,
    normalized_path: &'static str,
}

#[derive(Clone, Copy)]
struct ModuleGraphFileAdmission {
    path: &'static str,
    source_sha256: &'static str,
    metadata: ModuleMetadataContract,
    requests: &'static [ModuleRequestAdmission],
}

#[derive(Clone, Copy)]
struct FixtureGraphModuleAdmission {
    root_path: &'static str,
    files: &'static [ModuleGraphFileAdmission],
}

#[derive(Clone, Copy)]
struct ModuleGraphRootAdmission {
    path: &'static str,
    closure_file_count: usize,
}

#[derive(Clone, Copy)]
struct ExactModuleGraphAdmission {
    root_path: &'static str,
    files: &'static [ModuleGraphFileAdmission],
    closure_file_count: usize,
}

const MODULE_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: None,
};

const MODULE_FN_GLOBAL_OBJECT_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &["fnGlobalObject.js"],
    flags: &["module"],
    features: &[],
    negative: None,
};

const MODULE_GENERATORS_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &["generators"],
    negative: None,
};

const MODULE_IMPORT_META_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &["import.meta"],
    negative: None,
};

const MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["import.meta"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_IMPORT_META_ASYNC_ITERATION_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["import.meta", "async-iteration"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_IMPORT_META_DESTRUCTURING_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["import.meta", "destructuring-assignment"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_IMPORT_META_OBJECT_REST_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["import.meta", "destructuring-assignment", "object-rest"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_EXPORT_STAR_NAMESPACE_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &["export-star-as-namespace-from-module"],
    negative: None,
};

const MODULE_EXPORT_STAR_NAMESPACE_FN_GLOBAL_OBJECT_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &["fnGlobalObject.js"],
        flags: &["module"],
        features: &["export-star-as-namespace-from-module"],
        negative: None,
    };

const MODULE_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: Some(NegativeMetadataContract {
        phase: "parse",
        error_type: "SyntaxError",
    }),
};

const MODULE_RUNTIME_TYPE_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: Some(NegativeMetadataContract {
        phase: "runtime",
        error_type: "TypeError",
    }),
};

const MODULE_RESOLUTION_SYNTAX_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: Some(NegativeMetadataContract {
        phase: "resolution",
        error_type: "SyntaxError",
    }),
};

const MODULE_FIXTURE_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &[],
    features: &[],
    negative: None,
};

const MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["generators"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_EXPORT_STAR_NAMESPACE_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["export-star-as-namespace-from-module"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_LET_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &["let"],
    negative: Some(NegativeMetadataContract {
        phase: "parse",
        error_type: "SyntaxError",
    }),
};

const MODULE_LET_CONST_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["let", "const"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

const MODULE_NEW_TARGET_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract =
    ModuleMetadataContract {
        includes: &[],
        flags: &["module"],
        features: &["new.target"],
        negative: Some(NegativeMetadataContract {
            phase: "parse",
            error_type: "SyntaxError",
        }),
    };

/// The complete pinned declaration-position parse-negative cohort selected by
/// the two natural import/export filename families. Source, metadata, and the
/// filename-derived 43/43 partition are frozen by
/// `scripts/generate-test262-module-decl-position-a.mjs`.
const DECL_POSITION_MODULE_ADMISSIONS: [DependencyFreeModuleAdmission; 86] = [
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-arrow-function.js",
        source_sha256: "f40cb30b08cbb5ef6457ccad910e7decfe3185f2eaa1a287c180238249484b08",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-block-stmt-list.js",
        source_sha256: "9b9121f54dec42011053db4bafbd35a9b34533d9aa491449bda81a953dbc8b8e",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-block-stmt.js",
        source_sha256: "672b0286eac70d63e6947973efe993889bb0c561e2aa912f484703121a74b644",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-decl-meth-static.js",
        source_sha256: "d95d5f2c7124efa8dbb3f9a1dbea0c89288d4fe3ed922b52c381d97c2cba36d3",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-decl-meth.js",
        source_sha256: "2702e8175d968af6812c9b1908c85589604c01c76300c4301d0ad854c35c5837",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-decl-method-gen-static.js",
        source_sha256: "999907d4e92e1202550a291c08758661fbecacd292b59d4573ca0008b849ecd8",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-decl-method-gen.js",
        source_sha256: "38062e032b642b33af5a2abaa150129af9e92aa3b088bfca0a3b28fa9ba682ae",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-expr-meth-gen-static.js",
        source_sha256: "6bebf2ee3a7b0cde12ae758d1db7744526ea7e8bd5945532302b1f17e29337be",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-expr-meth-gen.js",
        source_sha256: "6088e65e1d851175f25a6a0b9179f299fd8f3850c8acf33206c08555569042c3",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-expr-meth-static.js",
        source_sha256: "7bbeca7b791a54e260e74b9b09489ed5c89259c22ee46778b2694927a4e43181",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-class-expr-meth.js",
        source_sha256: "f9e2478786dd29037564802af92888e24a75a00e6b17ffe2ef7ac327f94bef24",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-do-while.js",
        source_sha256: "03f3a82dfef2663a8789889bb91ed4a5f88366bc6054311488f6ac9316d947b6",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-const.js",
        source_sha256: "6f03002ff9645f4fddf352fe9f755ed559a7771612a732711a5cbf0c5f064b96",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-in-const.js",
        source_sha256: "0c032333f847b5146e03ea58af7bffd56becb6692d447b55b41a62c074c39d0b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-in-let.js",
        source_sha256: "98b67cc4ea08693663dbbe67a5e0f60ee231932e7d95db251ec1e831032828bb",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-in-lhs.js",
        source_sha256: "d7af620c52aa277dc309fe723fbce73653c4ed4d1289e92512e2a32fb1893dc5",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-in-var.js",
        source_sha256: "8badb79bc86208a3fc85332d70125202c91dc3c2cc2b364d14f575aa3e92f291",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-let.js",
        source_sha256: "320b67dab853b0e2039cb21878967abe295d134f688e550564f850ad1641d761",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-lhs.js",
        source_sha256: "6090a5f964bc59edd0ee3a615a3f0911bdc21dc0c83b751c3444b00d4f512f34",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-of-const.js",
        source_sha256: "b9ef024ed8440a85ac08c3d72d3bcb0696f6bf19103156a89770c51e266f2d77",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-of-let.js",
        source_sha256: "eeff61837e4904633583ca37f8762132f291d55bcf7467d97e6cc2e0beab7d64",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-of-lhs.js",
        source_sha256: "be4cabd5028e6d6e307849cbde88c18e289d1c13131d3865787f033e6f0581e6",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-of-var.js",
        source_sha256: "cfda820f094b9bc41021a485356527d4d3058b1394a89a40794dac2f7bfaa24a",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-for-var.js",
        source_sha256: "7b9419c84e7bf2de54cb2fb09c1bbbd19ff6e18d8ae8228959710e69a6440a65",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-function-decl.js",
        source_sha256: "dd514b21b5b3d1efda22f89f95abb9ee3c53c9a98a14078fceac4194e8f0b35b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-function-expr.js",
        source_sha256: "52cddc21d492109d11a1961ac45622bb334e7c0aeb03b6379c7e990b0ae8dc35",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-generator-decl.js",
        source_sha256: "253300aabceb53e210d66cd996d9133cf92785eb753071d03a5800ccff928eff",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-generator-expr.js",
        source_sha256: "42583588a5fe403080bd0472717c228c6ed87f184c10b5f74fd86650bc9de0af",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-if-else.js",
        source_sha256: "11637912575c0476877fa4bcee226f6bba740def26e905478c3b4da937a7b42c",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-if-if.js",
        source_sha256: "1af08c701e665927515ebffb4271e429cd7bb23870bb024d750e590d7c31a9e9",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-labeled.js",
        source_sha256: "3a04f8b3b45adda66a448476401deb86facf0741f05a5a6bc06b82eedfbb85b8",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-object-gen-method.js",
        source_sha256: "33699c38f19f2d37ee9fe3d6a52a3fa5506ff44db49c7f7ba66abd4115b2f11c",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-object-getter.js",
        source_sha256: "6361a65f77e29a919367450ccfa6137a0ac2993c9b9794d062db5bb9a3a77d7d",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-object-method.js",
        source_sha256: "cdb94f604834a9d06ba622c98052aae0c4da8b68c5337502caced561396b5663",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-object-setter.js",
        source_sha256: "8387154afb4ace34b08a6d5f869274b718ec050a906b821725e43c9e719ff8d5",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-switch-case-dflt.js",
        source_sha256: "a6b483ba9aa25157f7dcb3f43df4e9c7f8d8a942ad78e11c7dd4b73b566bfff4",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-switch-case.js",
        source_sha256: "29228932e7a4d943b2aad870a48c37493ebb81ec50c36fc8a2a2bee03f0f15b9",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-switch-dftl.js",
        source_sha256: "519daa3e06db9bce77d591a9818d3cf4dac296e3a58f3800132745dd5e2d0c73",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-try-catch-finally.js",
        source_sha256: "2d7f5a05304ba7dbfb66c6f811070ab1bc679066cc056269fb8825b58f545100",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-try-catch.js",
        source_sha256: "aa0cb27846dec10af3526f732f74d3d6b349dc79651fd5a2059cd841a0a088e6",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-try-finally.js",
        source_sha256: "ac0b29087da49f4479f1f1c1a1926675d515fc31332f488a4f827f033124f400",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-try-try.js",
        source_sha256: "3a568259eecfb0598f8c6305b7664e6ddae1af03fdfa1670b4c8e1b9e4828232",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-export-while.js",
        source_sha256: "674fe906049b9e85b6e04795c4589b199b70e4cc2425a3ab01af698c0ba8d89b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-arrow-function.js",
        source_sha256: "7464f0c2346f5bc39d0b2a081ef40b190bf4c4902aa47fff84efd021a9b5ac0f",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-block-stmt-list.js",
        source_sha256: "b09db37247c97fb96c84dd164201569c2b67dac65b689786645825c6bb619c36",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-block-stmt.js",
        source_sha256: "aa8ed707204e405c64c6ae59b3335c2d56e192ad09ba264b1852ae9c58e1b983",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-decl-meth-static.js",
        source_sha256: "8f21921c9bd146e06b8a3886c1ae1dbc4ec734bad94feff3b901d124a2df525f",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-decl-meth.js",
        source_sha256: "d7bc0622f0fbcd5dc658c7f1050ab924213ad4b5c21f72b8b1ee298479173fc6",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-decl-method-gen-static.js",
        source_sha256: "b073223ff067ada91d48cc9c0fe28cff4c0f41c575cebc6a20bbaa247092f450",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-decl-method-gen.js",
        source_sha256: "37808143dc7555a443256b6d81924ca099bf54850bad08a944c1cc578228d538",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-expr-meth-gen-static.js",
        source_sha256: "e3aafffee8b0932bd2c784256cd48d6fd5fcc71d36f4560c648b82ceb60f8b76",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-expr-meth-gen.js",
        source_sha256: "6dad1bcc3d0c8a984aeda7a9d7bbebfbc09302c198e3da5a183c243b5b6b8d1d",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-expr-meth-static.js",
        source_sha256: "bb141d26c7fa3a7a21566abe253b1420a563c08a93f244787d355d4b3fe3275b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-class-expr-meth.js",
        source_sha256: "8fc56be322e4b8c550c80138e0c6f457cb076f085b2a92a3a2626cf116392987",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-do-while.js",
        source_sha256: "377b795da559280f58114f84dcebd0d1fe8d7a2cbcddc107ca11cbf4c1246024",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-const.js",
        source_sha256: "a2360e23f781052e560ef97d6681e0377f6f859dfc1201b362cff76de022f576",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-in-const.js",
        source_sha256: "1c2b112a7341a1074aa21e8c7635cb7920cf4a3517ae0bef9aa02920f9818c79",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-in-let.js",
        source_sha256: "6d5517f5d5718b12275a4a1766daa1301cb457732e3bf826d067fe7528d08b4b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-in-lhs.js",
        source_sha256: "d5572270fcd20392cd29f4c8cb49438b979e124aeaefd4039fb602a79f66fd94",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-in-var.js",
        source_sha256: "f6829e8fc4ea8f8222d0a9d5f182ed13b2dd64f6dd804f2451c88ba95044d4a7",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-let.js",
        source_sha256: "69415ec4642192d32d664966a5ec816b3a390d2a0deea2e3c2520d69c6ede986",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-lhs.js",
        source_sha256: "e1d79087c1588f7496f6724c309f07eb4cc09851b159e0736fac76971a481111",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-of-const.js",
        source_sha256: "a374c6a7ca512cb77b5ef1aead876efccf86c36d0961037e80c3e997c67b5c03",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-of-let.js",
        source_sha256: "703259aea5c83ab5e92df77379a5918ba34affae6d643bd6488c5e2c084d1e26",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-of-lhs.js",
        source_sha256: "2d9050b67200ac9bcfe4741f028a526ed68b694e0fa5a4da4fa4e5c4150043dc",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-of-var.js",
        source_sha256: "6f1a620011275b58112ae5d5fd8b45c77659894bd3a38267de2bc4d1c1ff5322",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-for-var.js",
        source_sha256: "38fc2e55f93a932e61a33f2451940302b9fbc4b696d62e03756ca9ec2f2229f5",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-function-decl.js",
        source_sha256: "3251561f8947e8e1cb5cb41b971498238a93a6ccabe5ba767a677ed56315e127",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-function-expr.js",
        source_sha256: "82df99af92d5b72ab0e9c15f097645691280697e290b8152862266d18fbee03a",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-generator-decl.js",
        source_sha256: "c0c635ccd90df35de5d5057ac02884151f5a4364be5bd24e530a8a0b7e317b29",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-generator-expr.js",
        source_sha256: "87505f3264569c5434dd6a47067eac982e695540c7e3d83584ae318b53f49906",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-if-else.js",
        source_sha256: "938da0b8f3698c56e20368575c7c1b5dbe7e79070cbb178a4d9ca4930fe93d78",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-if-if.js",
        source_sha256: "66a59fa74993d9a5d689b0b75d593930d2643aec2432ee04ce4661adc73c0ac8",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-labeled.js",
        source_sha256: "1d48d676f41c18e9cb8db64871c2a36d22c3c9c218e90aef713b8edd9d1e26ef",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-object-gen-method.js",
        source_sha256: "b62e49f7d57eff2a2254f0b4477e30aec5e967e55857dfe09545cb52d27224c4",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-object-getter.js",
        source_sha256: "b88c8027d36d409f34bc0bac702bb1a069dfbcba3cb2399deab101b26ca8a5ce",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-object-method.js",
        source_sha256: "44ead756fae51763fb46ffb54f2ed5b2adf4bc2845fc9da19e82da8b803e68e2",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-object-setter.js",
        source_sha256: "98f35df77aa0f663054a229873e2f4cea1ff466c1e8da8f94537e2c22f4657b2",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-switch-case-dflt.js",
        source_sha256: "2af707bc0ad27b1a298eea1698b30086baf1b85069bf63ab7b9eafd535ba19fb",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-switch-case.js",
        source_sha256: "af3ffa2f1a2901115e0c4d25f490bab2b006a66cbd6367a35495764df0654e9e",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-switch-dftl.js",
        source_sha256: "90700e40c5e5a06ab776ba70e10ebbd08bcdf35b21b70acf1dc9718f190e8faa",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-try-catch-finally.js",
        source_sha256: "ca1e5bc814b4f60916aa050264bab37435bd69f23cacca3413a6cd5def1e7826",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-try-catch.js",
        source_sha256: "d15e6be87b02aff9c55de0915b1ab54caff359349f084b1b05e3effa9dfaac44",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-try-finally.js",
        source_sha256: "f90bf492891ca47c40fb1948902522d152489716fdf0ef328f175f8f6e1dd363",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-try-try.js",
        source_sha256: "a5cd4624b3b2f5dd032a2f4f8a2aec43241bddc5739198a37878a393b446d055",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-decl-pos-import-while.js",
        source_sha256: "d06ef1fedc6b78317a7c0a2067b1e11096b3844ea6a12231a0c09de5aeede9c6",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
];

/// The remaining pinned static-module parse-negative frontier selected from
/// the full suite after subtracting every previously audited negative. The
/// exact selector, source/frontmatter digests, request-shaped parse canaries,
/// and adjacent unsupported syntax are frozen by
/// `scripts/generate-test262-module-static-negative-a.mjs`.
const STATIC_NEGATIVE_MODULE_ADMISSIONS: [DependencyFreeModuleAdmission; 67] = [
    DependencyFreeModuleAdmission {
        path: "test/language/export/escaped-as-export-specifier.js",
        source_sha256: "d49a0f074128a9ad3f84655b12d9332a8a3df9cf40bd53545b43330923c902b5",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/export/escaped-default.js",
        source_sha256: "b8bb67c1db599f90eecf75542bccf4991e71aadcdd969a33d00ff166fbcc2b60",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/export/escaped-from.js",
        source_sha256: "25aa13b6d9c98f872c1b90a92f1188dba7365a2ddef2cfb671a19bcd5e6d3079",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/expressions/class/class-name-ident-await-escaped-module.js",
        source_sha256: "2a47a664d19373761ab9ad843961dea1dc614b88e422133016a1184bfadd4f75",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/expressions/class/class-name-ident-await-module.js",
        source_sha256: "20bf0d767ef141072f3a6f5e854248c18d536cd11e723a3ff111eb67515c8000",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/import/dup-bound-names.js",
        source_sha256: "713377cef5d18a264594cc81330d0c2d400c4a49d435be7d8585747188cc2519",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/import/escaped-as-import-specifier.js",
        source_sha256: "5df8fa3fd6b4e09fc6e450056073a2ab49e583f41ab7b5d8f313ebd66e28c846",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/import/escaped-as-namespace-import.js",
        source_sha256: "ea1861604b38c5262c0fd324ef297fa63ea3584a116c72ed904fc3864c73ccfc",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/import/escaped-from.js",
        source_sha256: "caee3ce2bfcbd50910856d98488036d93a252a1a5962ffc75fa2c0a7fac806de",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/comment-multi-line-html-close.js",
        source_sha256: "fc3fae0f513b1db887ec33ec7fc35fe698248bdfe20df2cc6380592033080faf",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/comment-single-line-html-close.js",
        source_sha256: "3b43fff901f4d1cac508995fdc369375b621ce3842fa29cc2f39cb9ae51524c1",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-as-star-as.js",
        source_sha256: "96fc4545bac6f801fe346217784f6bca9e31c1f21025a26c665b395212f9e541",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-decl.js",
        source_sha256: "32ecbf5b749af83757b28c851d8f98ed9b9d014a2025eb5d200ba0525899e5af",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-dflt-id.js",
        source_sha256: "60fa7477ce1dd373895d75ef3049f495b5f138ebec5e477f3dc6d724dea91183",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-dflt.js",
        source_sha256: "4e3b96290553bee9582fcbb7f341f50aef504f8787a31d184b0cb97a1fb3f4e2",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-id-as.js",
        source_sha256: "4ea88ead4c37cc05d806cadfa02dfa171ace946959d6031a22adbbb37a165cc3",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-star-as-dflt.js",
        source_sha256: "87d3bbb5dd4d155e192e6c4f96679ca4593e95c42174d6d8bb2bda70fe6725d5",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-lables.js",
        source_sha256: "7e0e0bd8fa857e82af0745ccc4a641523d5609039fe309f07288d892bbbc4616",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-lex.js",
        source_sha256: "972107d1b37f59ff142b00f4eab902f80dc2d09e2484c896695da1ddf6376caa",
        metadata: MODULE_LET_CONST_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-top-function-async-generator.js",
        source_sha256: "2568ac9f9b933ff2f796736d78268fd3d744779afe65eb457c4ea0f1b017954f",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-top-function-async.js",
        source_sha256: "bc4fda16ce29bca61ae5da38666714f4a3a23bd7ea45cd6946a23a58606c4c3c",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-top-function-generator.js",
        source_sha256: "0790cc57a63c540f475fe95ddaaa9593089079d61ab5a57042ec1793bbaf5906",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-top-function.js",
        source_sha256: "a31b3adb15b94dd1a5411511e73c4c3d1fba61783d44e86703a852fea726d01d",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-export-global.js",
        source_sha256: "93d50d0d347b0dc2c2e25a7d57a093b166b358f83da7afb36091cea3b73242c4",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-export-unresolvable.js",
        source_sha256: "b1fd3fd7568179bb4dff65c65bab75feaac235b933ee0c048062c3091ca7df25",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-import-arguments.js",
        source_sha256: "f7bd4a8d7f839d89ed84922588b5242f5bdd9d9e01114ebe2b1dbe827cc8fd50",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-import-as-arguments.js",
        source_sha256: "d2761605cdd1657cb54fdbca1b45d9d46a6a6063523e884783c2c80909cae753",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-import-as-eval.js",
        source_sha256: "742fbd6917df4cb1fe5e0058504c83afe15ac2588ef370977962672d61ccf230",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-import-eval.js",
        source_sha256: "8cea44aabd764438b542ff89ec3696fa2ecc12506add61a2d07723bc64d32788",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-lex-and-var.js",
        source_sha256: "cc69022adc5206e0eea03f5588579e81ade90693598e397384454c280e9f456a",
        metadata: MODULE_LET_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-new-target.js",
        source_sha256: "4219adeb88786bf138e92c6472e2b7f3cd20f0d775950d5136c9570b29f9daa1",
        metadata: MODULE_NEW_TARGET_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-super.js",
        source_sha256: "183fb7d0634760339e7e79e32930a1ec6e154d03397b97c353beeda53b17f6aa",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-undef-break.js",
        source_sha256: "aacd79e538054f0473434091f3a658bccbd67a025fb7b28d55293e129e2114de",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-undef-continue.js",
        source_sha256: "a61065224aa4649b3a7198c9bee2dee1b29584c3e077eabca993404130863f09",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/export-default-asyncfunction-declaration-binding-exists.js",
        source_sha256: "d98193819df758ee15af97f1c4d23c2f8fa9a4edb185269c8404627d5a45f521",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/export-default-asyncgenerator-declaration-binding-exists.js",
        source_sha256: "ee3fed6aa4c9e83681025cc99633cb8f7b98e2b77c5cfb16aa7435a0fa835095",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/export-default-function-declaration-binding-exists.js",
        source_sha256: "bbe2acaa3668ea6b180f8da0ec42b303896a6e5cb6bcf2a61e954a8e0ab30d91",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/export-default-generator-declaration-binding-exists.js",
        source_sha256: "ab8509b914794616ddbf8c86923d2378243c2f33f5ba28b96369139ee6052cf8",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-export-dflt-const.js",
        source_sha256: "741df6cfa5fb8914617c5f772dbe9d0b05195d321692183dc59f99433b44030d",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-export-dflt-expr.js",
        source_sha256: "7c548ba4e9d7884d67be5bb80113c6be043ce535a4bb2c89e294e10d19ea185a",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-export-dflt-let.js",
        source_sha256: "fd5262ec065a167f874ade7a3e7b47a0797d3020e5b271848a0aacd78e510139",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-export-dflt-var.js",
        source_sha256: "15b4beeaaa05b262340b9e6fc952f6d8a07afa4621d6f0bcbfbb7b2c40a5a2cc",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-hoist-lex-fun.js",
        source_sha256: "9d340acc95e544197f78a4426019d855ad8b82326326cdc8b806a05eae08edc8",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-hoist-lex-gen.js",
        source_sha256: "aa619a56e408c433fe25c41a3ccd7069b554bace0cdc37593d2f16952bab94e4",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-invoke-anon-fun-decl.js",
        source_sha256: "2b2e40e0ef3ad446cd58b0bff201d510478fd84ab337be133037a4c53c83b8e4",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-invoke-anon-gen-decl.js",
        source_sha256: "a52c94752e16f24ec162fc7faa7ebe1c52e82877667a340b6846528eb020be1e",
        metadata: MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-return.js",
        source_sha256: "2974d66d5ce1676c234e6d7d4ad909a4d41dbba3842e6fbec335010062d8a8de",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-semi-dflt-expr.js",
        source_sha256: "7e4ca7c13a3c4707a4d1f8c1527f3a59c91a4b171aef7fb55aa47ce036076e82",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-semi-export-star.js",
        source_sha256: "687ef880aaf279bc309e211836b7e0cb7237e448a155e46bc29e9fa5a62671bf",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-semi-name-space-export.js",
        source_sha256: "b87f75e52e0071380668d3dc4297f20aab31f4949ed43f30564ef86650f21f54",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-semi-named-export-from.js",
        source_sha256: "abb1fcd8e0960fa00473ae3a28266df21bf8cd91dc93464a4a018d9cfaab246d",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-semi-named-export.js",
        source_sha256: "d99ad4930ef415ca9a1b693cc4f1a48f415b4f63c4b4686f477398ba398a5b03",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-syntax-1.js",
        source_sha256: "9f10e1bd0f7207a4bf9bef31d1f39f0f7d026692b968ef1f804276bacb80ef2b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-syntax-2.js",
        source_sha256: "af3a93ff480f49725ef6d8311754a8d5cfe9997ced28a7f22a40f5c400ab9b21",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-err-yield.js",
        source_sha256: "12ccea4f76e1fdcb62b1ab4171d723d9a8f74284ce1a268bdb88d456fa14f2ad",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/reserved-words/await-module.js",
        source_sha256: "44edf8299f089ae5a58c536e57d8796867171da8237c93c6e28a752661c86a62",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/statements/class/class-name-ident-await-escaped-module.js",
        source_sha256: "00f02236fb308fa6e6a14ae2bc38d09e7ddf85437d915f60e884d947d8dd59c7",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/statements/class/class-name-ident-await-module.js",
        source_sha256: "ea1e0387dca386e8ba340eeee172d1f79e1586908e3a5c833c2d9375add9f29b",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/statements/labeled/value-await-module-escaped.js",
        source_sha256: "495a2a433b655c08b6035b81023f624ee6a17817f3b97dec99955c0853c6bd42",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/statements/labeled/value-await-module.js",
        source_sha256: "39de9daf4b36b4f9644b1b92a5203d306d8ad0c9f70d82f6467713d9149c02d9",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/async-functions/async-contains-unicode-escape-module.js",
        source_sha256: "1e867c1b31b5c1d2de9bbc9602ccda4d0b1ccda97a891e67f370e4fde2c2bb9a",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/fields/await-identifier-module-1.js",
        source_sha256: "f1e499ee086e8aad54d60df9bdee33f7e5e272d18773c014bc611b696867e80d",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/fields/await-identifier-module-2.js",
        source_sha256: "141a03470fbb0faeb21bb06f386d20618ff698145c5506a0096e5492d038a802",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/fields/await-identifier-module-3.js",
        source_sha256: "c53782f32159445d24f3e136feaa2e100d1db2e3c0e69f7baa6db25902dbc673",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/module/await-restricted-nested.js",
        source_sha256: "7c18e2a04b04deeed814b78b7103f19889be6c95a55562f4851a9d88f151848d",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/module/duplicate-exported-names-in-single-export-declaration.js",
        source_sha256: "f29db520cc3ef595bf2d581992c3bf8a353ef2069a3e4181f6ecb548fddc48a8",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/staging/sm/module/duplicate-exported-names-in-single-export-var-declaration.js",
        source_sha256: "153b0bfe37c6f8f38f82f8f6a2138f4cd455de1c3aaf48b7e05c4c2ea2386ec3",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
];

/// Source- and metadata-authenticated dependency-free module roots admitted by
/// the first static-module Test262 milestone. This is deliberately not a
/// general module capability switch: every other module retains the
/// `unsupported-module` selection result.
const DEPENDENCY_FREE_MODULE_ADMISSIONS: [DependencyFreeModuleAdmission; 13] = [
    DependencyFreeModuleAdmission {
        path: "test/language/comments/hashbang/module.js",
        source_sha256: "5fe73a40369e7cbd61f4061b027c9b508d6f1752fc83b29a4f1e4af7e8471926",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module", "raw"],
            features: &["hashbang"],
            negative: None,
        },
    },
    DependencyFreeModuleAdmission {
        path: "test/language/eval-code/direct/export.js",
        source_sha256: "648a257196bc895409842b12191cc0a8d9e10d28e66886afb89059412761caca",
        metadata: MODULE_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/eval-code/direct/import.js",
        source_sha256: "28c29caa8c8649579790526b511323df04837efad886d2f9d0ea75140dc5fa89",
        metadata: MODULE_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/comment-single-line-html-open.js",
        source_sha256: "789641728f7d8496801f145059d329c8b3c9cc1d2901ecbe893ff70e5e426d11",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-id.js",
        source_sha256: "c113c88cba6a99ba5ef7cf1c4c503c60d374aad2f6de2a3a112d6d1be937d91a",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-strict-mode.js",
        source_sha256: "a72ab52b0625b5becdc0a4f7e4945848582dd797493b4590b7a2ea25b63dd4e4",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/eval-self-abrupt.js",
        source_sha256: "a593ac28375f793312830e40cdda392054352f4fa692446de8ce2896c4518aa7",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &[],
            negative: Some(NegativeMetadataContract {
                phase: "runtime",
                error_type: "Test262Error",
            }),
        },
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/eval-this.js",
        source_sha256: "044874d01e501861c9c1d451ddd67e1c224a768045be75c5c49e0eb182d998c2",
        metadata: MODULE_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-const.js",
        source_sha256: "a36eaed3d56e39769c951b6ca041e22a9cbd1aea1e5dc3651f416992815dca81",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-fun.js",
        source_sha256: "92b10ca365a70fb2a9b4ba5e98add3e14912e6dd14c9271ee7a88f157945f784",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-let.js",
        source_sha256: "fd0c09f7adc72c46b66fa440450bbaa5db68173c09e8b6f54af58978c27f99ac",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-var.js",
        source_sha256: "8f9e41100266ea157c23977f9cb6646ec9b8c826362d2c8eaa44b5b1c2ba232a",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-export-empty.js",
        source_sha256: "eccb82249ee01600351841616110a7e8182e7056561f6eb9e44120b7aaf73cd8",
        metadata: MODULE_METADATA,
    },
];

const EVAL_GTBNDNG_INDIRECT_UPDATE_FILES: [ModuleGraphFileAdmission; 2] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update.js",
        source_sha256: "2e382b6cef4a65f3c1b58ed7a21f9311b2627e7980b410805d1018b714d4b5b6",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-gtbndng-indirect-update_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js",
        source_sha256: "86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

const EVAL_REQUESTED_ABRUPT_FILES: [ModuleGraphFileAdmission; 3] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-abrupt.js",
        source_sha256: "96266e78b158e46ce04ab22c987e62a4ff5c6b9484ebb8adacd993f44e4e8f29",
        metadata: MODULE_RUNTIME_TYPE_ERROR_METADATA,
        requests: &[
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-abrupt-err-type_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-abrupt-err-type_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-abrupt-err-uri_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-abrupt-err-uri_FIXTURE.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-abrupt-err-type_FIXTURE.js",
        source_sha256: "ce3ebfa86081c793bf36a681e6f1e4faca99e529b338bfbfc433b550e1bf27e8",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-abrupt-err-uri_FIXTURE.js",
        source_sha256: "e6bbf1d0467c9a361289d3d6a40ae8479bff3c7d928b10140c2171b309207572",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

const INSTN_RESOLVE_EMPTY_IMPORT_FILES: [ModuleGraphFileAdmission; 2] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-resolve-empty-import.js",
        source_sha256: "88161e79a99ef0372dddb122e6dc2e545961bf0d4775f53ba48531b3fcc3fadb",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-resolve-empty-import_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-resolve-empty-import_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-resolve-empty-import_FIXTURE.js",
        source_sha256: "d019396c51ec65b57af8edc64bcc7b969df709c1f0a11a6b5220bc5f09545e80",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

const INSTN_SAME_GLOBAL_FILES: [ModuleGraphFileAdmission; 2] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-same-global.js",
        source_sha256: "564f38753491b84941656868c73ce342c2111fc9b29ed7b681ee9732f4e5cbce",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-same-global-set_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-same-global-set_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-same-global-set_FIXTURE.js",
        source_sha256: "ac117f0e7632295f0e7b67bace1d65b72e2f4d9a3dd2b66643b3b27d24f48f8f",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

/// Source-, metadata-, edge-, and recursive-closure-authenticated module
/// graphs admitted by the loader/linker Test262 milestone. The four roots are
/// intentionally independent, so their nine total source files form the
/// smallest useful direct-import cohort and no unrelated fixture can be
/// reached through the worker loader.
const FIXTURE_GRAPH_MODULE_ADMISSIONS: [FixtureGraphModuleAdmission; 4] = [
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/eval-gtbndng-indirect-update.js",
        files: &EVAL_GTBNDNG_INDIRECT_UPDATE_FILES,
    },
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/eval-rqstd-abrupt.js",
        files: &EVAL_REQUESTED_ABRUPT_FILES,
    },
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/instn-resolve-empty-import.js",
        files: &INSTN_RESOLVE_EMPTY_IMPORT_FILES,
    },
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/instn-same-global.js",
        files: &INSTN_SAME_GLOBAL_FILES,
    },
];

/// The complete natural Test262 namespace cohort at the pinned suite
/// revision: every non-fixture root below `namespace/`, plus the one adjacent
/// ambiguous-export namespace test. Roots share one sorted file ledger, while
/// `closure_file_count` freezes each root's independently reachable closure.
const NAMESPACE_MODULE_ROOT_ADMISSIONS: [ModuleGraphRootAdmission; 37] = [
    ModuleGraphRootAdmission {
        path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace.js",
        closure_file_count: 4,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/Symbol.iterator.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/Symbol.toStringTag.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/define-own-property.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/delete-exported-init.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/delete-exported-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/delete-non-exported.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/enumerate-binding-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip.js",
        closure_file_count: 5,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml.js",
        closure_file_count: 4,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-str-found-init.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-str-found-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-str-not-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-sym.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-prototype-of.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-str-found-init.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-str-found-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-str-initialize.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-str-not-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-str-update.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-sym-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/get-sym-not-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/has-property-str-found-init.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/has-property-str-found-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/has-property-str-not-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/has-property-sym-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/has-property-sym-not-found.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/is-extensible.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/object-hasOwnProperty-binding-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/object-keys-binding-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/object-propertyIsEnumerable-binding-uninit.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/own-property-keys-binding-types.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/own-property-keys-sort.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/prevent-extensions.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/set-prototype-of-null.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/set-prototype-of.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/namespace/internals/set.js",
        closure_file_count: 1,
    },
];

const NAMESPACE_MODULE_FILE_ADMISSIONS: [ModuleGraphFileAdmission; 48] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace-1_FIXTURE.js",
        source_sha256: "da063d7cea6c2ddfb33c462850ed97c90ade8d57bda9655debc8acc6d4cd63e2",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace-2_FIXTURE.js",
        source_sha256: "865336830a521eb03d82ef1579dba5fb7fb04fbc617fdba2259b90562c43364a",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace.js",
        source_sha256: "47239d1ac289c855e372dcfd86e15ddaa57be453cec4d32cc1edc45e2f8217e1",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./omitted-from-namespace_FIXTURE.js",
            normalized_path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace_FIXTURE.js",
        source_sha256: "dfdb499dcdedf15de650ad013f4ee0ce265ce58cc8a1cd23cedbd1778d91ae93",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[
            ModuleRequestAdmission {
                specifier: "./omitted-from-namespace-1_FIXTURE.js",
                normalized_path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace-1_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./omitted-from-namespace-2_FIXTURE.js",
                normalized_path: "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace-2_FIXTURE.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/Symbol.iterator.js",
        source_sha256: "03082b2bf4a1432a2e190f4973be5c463dbe9a757b49f5908ac86b0a45343e09",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol.iterator"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./Symbol.iterator.js",
            normalized_path: "test/language/module-code/namespace/Symbol.iterator.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/Symbol.toStringTag.js",
        source_sha256: "6315c6f95c2c060a0f21a777655a498a47c44efbdbd72939685829f98355dca8",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./Symbol.toStringTag.js",
            normalized_path: "test/language/module-code/namespace/Symbol.toStringTag.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/define-own-property.js",
        source_sha256: "490086320b7c5fb23f7ca4ebfad4a326e5fe49aa4259e628f1c3a1e52c7690c6",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol.iterator", "Reflect", "Symbol", "Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./define-own-property.js",
            normalized_path: "test/language/module-code/namespace/internals/define-own-property.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/delete-exported-init.js",
        source_sha256: "989744c43f64724c9957e490a87fb456acb764ed2ac7a6d7f24147f37aff84b8",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./delete-exported-init.js",
            normalized_path: "test/language/module-code/namespace/internals/delete-exported-init.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/delete-exported-uninit.js",
        source_sha256: "ead3d19b68a4f4631f933fed4cf3f42367c429f7540c2a6dc25baca17e3755e3",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect", "let"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./delete-exported-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/delete-exported-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/delete-non-exported.js",
        source_sha256: "5549137de62e046e20a0d64fb75a1ecdb83d7e5569f10012f4acc76678041209",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect", "Symbol", "Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./delete-non-exported.js",
            normalized_path: "test/language/module-code/namespace/internals/delete-non-exported.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/enumerate-binding-uninit.js",
        source_sha256: "e3749a663a14cbfbe71cca4ff2fbd23d255bc9869e3a63520744cc4237b7edf3",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./enumerate-binding-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/enumerate-binding-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-named-end_FIXTURE.js",
        source_sha256: "4f1ecf3ed3337648b0d453d223ad34f5a24f0df166561699864c008dd8d0b43c",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-named_FIXTURE.js",
        source_sha256: "ce8799937a6671573f4c0b2dcfdf46f8e595a23d9bdd5de6a307343d7970bc8d",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-nested-namespace-dflt-skip-named-end_FIXTURE.js",
            normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-named-end_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-prod-end_FIXTURE.js",
        source_sha256: "b9af329375f371cddbec3e46804af29bea7a2ac992b4181135021ab5f3939b59",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-prod_FIXTURE.js",
        source_sha256: "f4b066890b604a1fd83857d58dd0a74a09aa05f3127ae601909b109726492ef6",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-nested-namespace-dflt-skip-prod-end_FIXTURE.js",
            normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-prod-end_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip.js",
        source_sha256: "9a7eac9328403480e06b15cce9ac2279486bf2bbc036d19e5223380247984ca2",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["export-star-as-namespace-from-module"],
            negative: None,
        },
        requests: &[
            ModuleRequestAdmission {
                specifier: "./get-nested-namespace-dflt-skip-named_FIXTURE.js",
                normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-named_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./get-nested-namespace-dflt-skip-prod_FIXTURE.js",
                normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-dflt-skip-prod_FIXTURE.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml-1_FIXTURE.js",
        source_sha256: "11c902ba608e8100b8379ba450a4fc25a245f31cdea28a6385d1159ee78f8000",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-nested-namespace-props-nrml-2_FIXTURE.js",
            normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml-2_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml-2_FIXTURE.js",
        source_sha256: "87b004638b126edb359c44ec998fd34089510d80dd5ba4af680c0a73f2b5d732",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-nested-namespace-props-nrml-3_FIXTURE.js",
            normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml-3_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml-3_FIXTURE.js",
        source_sha256: "fc4a16120836dbd4685742b12fff11a5066d8ad431bebb4a5ac6830f2979b52f",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml.js",
        source_sha256: "74fd6103875b20199f6a4ff1c31b0fae154319eaa5be03816c33c6751c1097cf",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["export-star-as-namespace-from-module"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-nested-namespace-props-nrml-1_FIXTURE.js",
            normalized_path: "test/language/module-code/namespace/internals/get-nested-namespace-props-nrml-1_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-str-found-init.js",
        source_sha256: "bba174e662b5bc81f0b82f71a98cadd6faeac1e885cdb356b211954bd6276faf",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-own-property-str-found-init.js",
            normalized_path: "test/language/module-code/namespace/internals/get-own-property-str-found-init.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-str-found-uninit.js",
        source_sha256: "367055bde6f3dae3dfe050ef2785f16a321a83c52363a75e09fa0ebedbfb3005",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["let"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-own-property-str-found-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/get-own-property-str-found-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-str-not-found.js",
        source_sha256: "722c22fdceee8ad9c76918220203241e79e038cd8dc33df3c43a744620b6f2d9",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-own-property-str-not-found.js",
            normalized_path: "test/language/module-code/namespace/internals/get-own-property-str-not-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-own-property-sym.js",
        source_sha256: "5daefc7f5e487c260f2d78b8087e141c6f065940340ef6440a5577f7567db759",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol", "Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-own-property-sym.js",
            normalized_path: "test/language/module-code/namespace/internals/get-own-property-sym.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-prototype-of.js",
        source_sha256: "1a49c3befc8b4c820e8bbb4d2f3ec1e818afdfba0a633e634de7b8b274795643",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-prototype-of.js",
            normalized_path: "test/language/module-code/namespace/internals/get-prototype-of.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-str-found-init.js",
        source_sha256: "ddd4e6b68b4093df6f87b8c1977155cdc85526e568a721f16b7ab44363405c52",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-str-found-init.js",
            normalized_path: "test/language/module-code/namespace/internals/get-str-found-init.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-str-found-uninit.js",
        source_sha256: "fba153a816cfdfe812ecb222894dddbf0246a2220074b7aaec98d36e7cd8b4f8",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["let"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-str-found-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/get-str-found-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-str-initialize.js",
        source_sha256: "eb293f29acf96e5808797d3aec9964ef4ef9b629fa9c1675afd0c15c2a05a01e",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["let"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-str-initialize.js",
            normalized_path: "test/language/module-code/namespace/internals/get-str-initialize.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-str-not-found.js",
        source_sha256: "00adb485df5fb8ca302897d4db5d4b3e89a83ef569dd67913a12edc2e0db7b6e",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-str-not-found.js",
            normalized_path: "test/language/module-code/namespace/internals/get-str-not-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-str-update.js",
        source_sha256: "4acedca4f7c613fc1f15d855020f39dc673c508d1d7403db1f7ccadb3c6ad01b",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./get-str-update.js",
            normalized_path: "test/language/module-code/namespace/internals/get-str-update.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-sym-found.js",
        source_sha256: "954376d03885980563797915af1d933056d438d9eddab074a63cf4c35f2256d6",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-sym-found.js",
            normalized_path: "test/language/module-code/namespace/internals/get-sym-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/get-sym-not-found.js",
        source_sha256: "383f50910a9241dd148e178979e053d4b284cb663998d6abf1ccbf0eb51ac68a",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./get-sym-not-found.js",
            normalized_path: "test/language/module-code/namespace/internals/get-sym-not-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/has-property-str-found-init.js",
        source_sha256: "24244c2ffd67d73f90145b70b7f99df87c0962ba32679f057dad46a3ac098af8",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./has-property-str-found-init.js",
            normalized_path: "test/language/module-code/namespace/internals/has-property-str-found-init.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/has-property-str-found-uninit.js",
        source_sha256: "27471a2bf678598fec6c3df6fdfec8ed1236ef393cf931bb2ca603cdccf9216b",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect", "let"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./has-property-str-found-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/has-property-str-found-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/has-property-str-not-found.js",
        source_sha256: "5ea28c74bfe936f7dca93b9c1a678f213cacdeb57fb1499fb462e663c6259d0b",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./has-property-str-not-found.js",
            normalized_path: "test/language/module-code/namespace/internals/has-property-str-not-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/has-property-sym-found.js",
        source_sha256: "6572cf48501dae9f8d4602c60cb0d342c623965c5ec52a89a34238569358715d",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol.toStringTag", "Reflect"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./has-property-sym-found.js",
            normalized_path: "test/language/module-code/namespace/internals/has-property-sym-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/has-property-sym-not-found.js",
        source_sha256: "277e068c61a8c1d00e228147df1fadd2e699da65654e220cf2be652f98af7856",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Symbol", "Reflect"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./has-property-sym-not-found.js",
            normalized_path: "test/language/module-code/namespace/internals/has-property-sym-not-found.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/is-extensible.js",
        source_sha256: "c500c5b92cfd978223c8528fc64a783ff99931b1a9629011b95f5c3b6969048c",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./is-extensible.js",
            normalized_path: "test/language/module-code/namespace/internals/is-extensible.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/object-hasOwnProperty-binding-uninit.js",
        source_sha256: "e80c6a04e4f120c0b9ec63aa41013cc5de354476694abfebe1c0f0b5a171136b",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./object-hasOwnProperty-binding-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/object-hasOwnProperty-binding-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/object-keys-binding-uninit.js",
        source_sha256: "4d1566016d2096e5635f5fb02b0ce7777a030cba72131eadbb9436798ed2e06c",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./object-keys-binding-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/object-keys-binding-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/object-propertyIsEnumerable-binding-uninit.js",
        source_sha256: "d7c6352c00c55d094688435f1895aa74f0f5ffcf5c4325d9e072d905f6e448dd",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./object-propertyIsEnumerable-binding-uninit.js",
            normalized_path: "test/language/module-code/namespace/internals/object-propertyIsEnumerable-binding-uninit.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/own-property-keys-binding-types.js",
        source_sha256: "a3819b67e5dc41adf8c6d06ba9972d43f84f7f3a49d430f32a96aa98bfc8fc68",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect", "Symbol.toStringTag", "let"],
            negative: None,
        },
        requests: &[
            ModuleRequestAdmission {
                specifier: "./own-property-keys-binding-types.js",
                normalized_path: "test/language/module-code/namespace/internals/own-property-keys-binding-types.js",
            },
            ModuleRequestAdmission {
                specifier: "./own-property-keys-binding-types_FIXTURE.js",
                normalized_path: "test/language/module-code/namespace/internals/own-property-keys-binding-types_FIXTURE.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/own-property-keys-binding-types_FIXTURE.js",
        source_sha256: "3b4098e5e9b8e2e72e390b813ef3aa76417bb2bda217b631355da917e1d93c7f",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./own-property-keys-binding-types.js",
            normalized_path: "test/language/module-code/namespace/internals/own-property-keys-binding-types.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/own-property-keys-sort.js",
        source_sha256: "b0eae14038b0da50ab314e3b63352c650318657cfc6e56e4cc61342fb7824f0b",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect", "Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./own-property-keys-sort.js",
            normalized_path: "test/language/module-code/namespace/internals/own-property-keys-sort.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/prevent-extensions.js",
        source_sha256: "c0ee6e57a97cea72137fb19f80a12512fb2797f8903699a353ff717d6141bf30",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./prevent-extensions.js",
            normalized_path: "test/language/module-code/namespace/internals/prevent-extensions.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/set-prototype-of-null.js",
        source_sha256: "9c63575fc8841e01c02ef30ef47e7c8f713ad8cc4a879cf197f64b7023751e99",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./set-prototype-of-null.js",
            normalized_path: "test/language/module-code/namespace/internals/set-prototype-of-null.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/set-prototype-of.js",
        source_sha256: "3de9551554eea456e0075fe9dc03daeb5f8641c64d735f448d2c9396eff8e9b8",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./set-prototype-of.js",
            normalized_path: "test/language/module-code/namespace/internals/set-prototype-of.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/namespace/internals/set.js",
        source_sha256: "4cfe9d8bfa3dfe2ad3c663013855b43b40173b95f32bec06171813b79d56e87e",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &["Reflect", "Symbol", "Symbol.toStringTag"],
            negative: None,
        },
        requests: &[ModuleRequestAdmission {
            specifier: "./set.js",
            normalized_path: "test/language/module-code/namespace/internals/set.js",
        }],
    },
];

/// The exact top-level default/indirect module graph cohort at the pinned
/// Test262 revision. The natural root selector, complete source union,
/// request edges, metadata, and per-root recursive closure are frozen by
/// `scripts/generate-test262-module-default-a.mjs`.
const DEFAULT_MODULE_ROOT_ADMISSIONS: [ModuleGraphRootAdmission; 38] = [
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-anon-semi.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-anon.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-name-meth.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-named-semi.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-cls-anon.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-cls-name-meth.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-cls-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-fn-anon.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-fn-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-gen-anon.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-gen-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-in.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-fun-anon-semi.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-fun-named-semi.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-gen-anon-semi.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-export-dflt-gen-named-semi.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-trlng-comma.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update-as.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update-dflt.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-rqstd-once.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-rqstd-order.js",
        closure_file_count: 10,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/eval-self-once.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/export-star-as-dflt.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-iee-err-dflt-thru-star-as.js",
        closure_file_count: 3,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-iee-err-dflt-thru-star.js",
        closure_file_count: 3,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-cls.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-expr.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-fun-anon.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-fun-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-gen-anon.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-gen-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-named.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-star.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-err-dflt-thru-star-as.js",
        closure_file_count: 3,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-err-dflt-thru-star-dflt.js",
        closure_file_count: 3,
    },
    ModuleGraphRootAdmission {
        path: "test/language/module-code/instn-named-err-not-found-dflt.js",
        closure_file_count: 2,
    },
];

const DEFAULT_MODULE_FILE_ADMISSIONS: [ModuleGraphFileAdmission; 58] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-anon-semi.js",
        source_sha256: "8f51596535737bf33603d78543bfb00f8792802208b9ff0f9cc0212a7d329de1",
        metadata: MODULE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-anon.js",
        source_sha256: "aa9d5c379bff55735756c7f447ed368b56b0276277d97cca5a07c99b4cfc03d4",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-cls-anon.js",
            normalized_path: "test/language/module-code/eval-export-dflt-cls-anon.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-name-meth.js",
        source_sha256: "176ca161854205b96d780b0a5568008c5f98d3e85224417d00c1f8a2d9a226e2",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-cls-name-meth.js",
            normalized_path: "test/language/module-code/eval-export-dflt-cls-name-meth.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-named-semi.js",
        source_sha256: "bf7513623f7cc16522906e13371a8c6763bb287fefcc0a0901243c935640aba9",
        metadata: MODULE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-cls-named.js",
        source_sha256: "5522ec499f121872a462cc2c4b79572c84b96215818c194726c818ffb8595213",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-cls-named.js",
            normalized_path: "test/language/module-code/eval-export-dflt-cls-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-cls-anon.js",
        source_sha256: "b5fa8a12655de8b56d7a6c6c30c7686988acdc3b7f557052adb4d1ad9633c1d3",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-cls-anon.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-cls-anon.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-cls-name-meth.js",
        source_sha256: "4ed56bab4263eeabf56ab0227931f34dac658bb3ed3fb6e9e611eb100719cfca",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-cls-name-meth.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-cls-name-meth.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-cls-named.js",
        source_sha256: "b36534c4e4484c30c69ed52957405dc77bb694168a100b9c2df8da21e11bc2e1",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-cls-named.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-cls-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-fn-anon.js",
        source_sha256: "8d2ce3104e20aca09591911a65ad881cdcea15473b855662813d82be496bb89b",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-fn-anon.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-fn-anon.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-fn-named.js",
        source_sha256: "e2d8ba24e84aa368887535e974ca4c06c65137b097edd48438e1a8df75995b6a",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-fn-named.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-fn-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-gen-anon.js",
        source_sha256: "aa8991077c498d490869ec8003ca322036bc8769017a3b3bf3a1da5aef2d01a2",
        metadata: MODULE_GENERATORS_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-gen-anon.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-gen-anon.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-gen-named.js",
        source_sha256: "8bc314bf83d82152cb9505999b881ce13ce581445072d924fdd47483b17cb770",
        metadata: MODULE_GENERATORS_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-gen-named.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-gen-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-expr-in.js",
        source_sha256: "7551d897ea97bab6c708d0db6791c2da411bb16f279515758bcd14c6a5a45fba",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-export-dflt-expr-in.js",
            normalized_path: "test/language/module-code/eval-export-dflt-expr-in.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-fun-anon-semi.js",
        source_sha256: "5e07612c564201578fda64fcb97493bffcdb1a7d9d4731a1f937b22dda60bc50",
        metadata: MODULE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-fun-named-semi.js",
        source_sha256: "cdbef6d4d8e357472675cb56b0f9799a32035a3ebd1551169b533d5330d4c1a7",
        metadata: MODULE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-gen-anon-semi.js",
        source_sha256: "b4866570c972b626075e779a8ff3338437f55c95fd22910af9bf9095d1572ec3",
        metadata: MODULE_GENERATORS_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-export-dflt-gen-named-semi.js",
        source_sha256: "c8310adf20a006287f477bf98bef4487800950ad472a65b739e34ac0ba3fe88e",
        metadata: MODULE_GENERATORS_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-trlng-comma.js",
        source_sha256: "12f9d3c46c2fe9ac4b9607acc71e97973e6c63281c08524f335772e477db6f85",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-gtbndng-indirect-trlng-comma_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-gtbndng-indirect-trlng-comma_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-trlng-comma_FIXTURE.js",
        source_sha256: "5c22ed86c12987e7a3fbe9720539f5e5f2dab0e7e462b0a25b9bb74bb580c4f1",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update-as.js",
        source_sha256: "531a6d88591198d2c6d8ca9151a675b17b1259c150bfd0b24a9b5208799fcb52",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-gtbndng-indirect-update-as_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-gtbndng-indirect-update-as_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update-as_FIXTURE.js",
        source_sha256: "86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update-dflt.js",
        source_sha256: "2d949247244a5054173aff0bff717cb22d83048bd2e552e27b3a635fa3456544",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-gtbndng-indirect-update-dflt_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-gtbndng-indirect-update-dflt_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update-dflt_FIXTURE.js",
        source_sha256: "12a289fc58e54afefbf54b974ac7db0432b3dca1815d3c7f0fa7d4f639b0fe1f",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update.js",
        source_sha256: "2e382b6cef4a65f3c1b58ed7a21f9311b2627e7980b410805d1018b714d4b5b6",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-gtbndng-indirect-update_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js",
        source_sha256: "86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-once.js",
        source_sha256: "882ed0917cb6f3e51819d3df04b339fa3bedc5066f82e096a5caccee46b02ae6",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-rqstd-once_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-rqstd-once_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-once_FIXTURE.js",
        source_sha256: "7e63dfa9e539e07a14cb5ea9efe0e2a9c96d8779823ab37fe8ca40aba2211e00",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-1_FIXTURE.js",
        source_sha256: "4e8c98537b24278a6601d04bb1d2bbc58176b1c33d1d2d960bd70cc7bc16f900",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-2_FIXTURE.js",
        source_sha256: "5ffd70f38a902fdef8e02fbdd5368ad6ab32d8396dbc630a55a6cb5dac06b1b7",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-3_FIXTURE.js",
        source_sha256: "9f7e3c79460a2b7e4da418992c31252f2f1b56571d606b35ce98f40f25db0dcb",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-4_FIXTURE.js",
        source_sha256: "9a831329fea0311b153e2ba34dfafad10bb46a05d5bf42f40f30a9934cd0d376",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-5_FIXTURE.js",
        source_sha256: "e6c9b77d88e2d7bba9a4aa82dd410c82d54f9b753a7b794b3aed46b9700bebe9",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-6_FIXTURE.js",
        source_sha256: "98612128fbccba934f3db5cdc705ce56965555e31c28bd327dd4567dc56db835",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-7_FIXTURE.js",
        source_sha256: "8a5cb9176b931cb963c31f81c4be7e69528f9c354067990b6559363edeec8f73",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-8_FIXTURE.js",
        source_sha256: "2dcca213985438e5376357e19866341af0e5d78995d02a5631934f0c544be930",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order-9_FIXTURE.js",
        source_sha256: "aa255b335b9c9309ba4e630256f102a6bbaeae107905698cbf19c1ecb642e891",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-order.js",
        source_sha256: "6bc5fcbee5e8c6c9315ac71d6b88b47ff0b0c826a0f4430537130d64fd70f5e6",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-1_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-1_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-2_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-2_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-3_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-3_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-4_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-4_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-5_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-5_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-6_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-6_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-7_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-7_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-8_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-8_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-order-9_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-order-9_FIXTURE.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-self-once.js",
        source_sha256: "6639c2cb8e4ea955fc030d1811350f05b5c710f811904067a495e70c628f7581",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-self-once.js",
            normalized_path: "test/language/module-code/eval-self-once.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/export-star-as-dflt.js",
        source_sha256: "a9724b86d659a6d967194d51e68c42a8b22595eeb236ee0eda32ac57778156cc",
        metadata: MODULE_EXPORT_STAR_NAMESPACE_METADATA,
        requests: &[
            ModuleRequestAdmission {
                specifier: "./export-star-as-dflt_FIXTURE.js",
                normalized_path: "test/language/module-code/export-star-as-dflt_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./export-star-as-dflt.js",
                normalized_path: "test/language/module-code/export-star-as-dflt.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/export-star-as-dflt_FIXTURE.js",
        source_sha256: "630349bb17477e095abdc188b14a85a02bc228caa11f3a257f7f052148dff40c",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-iee-err-dflt-thru-star-as.js",
        source_sha256: "65ebe620436c09c8be4d37528ffa9d367d3419852af2b3cf285533d855c9f1cc",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-iee-err-dflt-thru-star-int_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-iee-err-dflt-thru-star-int_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-iee-err-dflt-thru-star-dflt_FIXTURE.js",
        source_sha256: "2c51cc1863b9a1c6b1043a6a46258752acf39fc2640733ce8d47895be318a986",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-iee-err-dflt-thru-star-int_FIXTURE.js",
        source_sha256: "52f6800be916aba38d70ec388015cb91686900ef40525123ce9bb368d31bd2a9",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-iee-err-dflt-thru-star-dflt_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-iee-err-dflt-thru-star-dflt_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-iee-err-dflt-thru-star.js",
        source_sha256: "c0d32ab7ca2682ce77e5cfefd9a72fcd371fc70c5a0ee553a9b710e5964b2f70",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-iee-err-dflt-thru-star-int_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-iee-err-dflt-thru-star-int_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-cls.js",
        source_sha256: "d27eee1aeb3d07af6844a2ddc798b886ea7e010f1837e4246925ccadd2804d88",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-cls.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-cls.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-expr.js",
        source_sha256: "fe0332bf39c1354b625d2f89abd5df929288ffaf3aa12d9f45003aea00244352",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-expr.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-expr.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-fun-anon.js",
        source_sha256: "375193b74f1be8dead8b515e8d83efee9ac5fe5c126253ce8059035ddafc793d",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-fun-anon.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-fun-anon.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-fun-named.js",
        source_sha256: "042384a9e73eac9d84d0894decaf5bc3c4be167293649c17f21f1f74b1945133",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-fun-named.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-fun-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-gen-anon.js",
        source_sha256: "e43f2c710106ec2059a660fcea954647460f5d92222ecb2c3d8dddef503a0a46",
        metadata: MODULE_GENERATORS_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-gen-anon.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-gen-anon.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-gen-named.js",
        source_sha256: "5f24d94650d27ba1c046b21e985c5d5b835db0ecf5cdb2679c70ae89b2fc1907",
        metadata: MODULE_GENERATORS_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-gen-named.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-gen-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-named.js",
        source_sha256: "6c2005b1dac577b28f6683ebe3a952c9cc5e0278c3148bab1477cc7d2cf3deb9",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-named.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-named.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-bndng-dflt-star.js",
        source_sha256: "ba471a14418389ae63bd62f70f30eb29b88ee7dd017505f3bf7db8c247d9b8e5",
        metadata: MODULE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-bndng-dflt-star.js",
            normalized_path: "test/language/module-code/instn-named-bndng-dflt-star.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-err-dflt-thru-star-as.js",
        source_sha256: "8969afda03d04ba65fb428016f4cfd070882b915bf5508b338d5950ca9a0cb07",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-err-dflt-thru-star-int_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-named-err-dflt-thru-star-int_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-err-dflt-thru-star-dflt.js",
        source_sha256: "7e8536f5deabfff2cf58214adfaabbd3b30dc9077e0700a9ea69a3841b04f81b",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-err-dflt-thru-star-int_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-named-err-dflt-thru-star-int_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-err-dflt-thru-star-dflt_FIXTURE.js",
        source_sha256: "2c51cc1863b9a1c6b1043a6a46258752acf39fc2640733ce8d47895be318a986",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-err-dflt-thru-star-int_FIXTURE.js",
        source_sha256: "2e50d471452af4e2edbdf2b8c5423677c1400cb6386d48dec493792ac7718131",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-err-dflt-thru-star-dflt_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-named-err-dflt-thru-star-dflt_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-err-not-found-dflt.js",
        source_sha256: "926ab4c8a1c02df68c146855e4facf94cdf0d0d8e20277a2ff446513a3460b5e",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-named-err-not-found-empty_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-named-err-not-found-empty_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-named-err-not-found-empty_FIXTURE.js",
        source_sha256: "2d326e77199c7b7def6df453731bddee1732fba0635344e7e47d161d7aa17dba",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

/// The complete natural module-goal `import.meta` cohort at the pinned
/// Test262 revision. Script-goal roots remain ordinary coordinator jobs; only
/// these 17 module roots need exact module-host admission. One root reaches a
/// single authenticated fixture and every other root is dependency-free.
const IMPORT_META_MODULE_ROOT_ADMISSIONS: [ModuleGraphRootAdmission; 17] = [
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/distinct-for-each-module.js",
        closure_file_count: 2,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/import-meta-is-an-ordinary-object.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/not-accessible-from-direct-eval.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/same-object-returned.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/escape-sequence-import.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/escape-sequence-meta.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/goal-module-nested-function.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/goal-module.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-destructuring-expr.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-rest-destructuring-expr.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-assignment-expr.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-await-of-loop.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-in-loop.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-of-loop.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-destructuring-expr.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-rest-destructuring-expr.js",
        closure_file_count: 1,
    },
    ModuleGraphRootAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-update-expr.js",
        closure_file_count: 1,
    },
];

const IMPORT_META_MODULE_FILE_ADMISSIONS: [ModuleGraphFileAdmission; 18] = [
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/distinct-for-each-module.js",
        source_sha256: "22972363efdffc5fdf15259186f9512509d5ebb4d18d67007c8535a3c5dbf0e9",
        metadata: MODULE_IMPORT_META_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./distinct-for-each-module_FIXTURE.js",
            normalized_path: "test/language/expressions/import.meta/distinct-for-each-module_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/distinct-for-each-module_FIXTURE.js",
        source_sha256: "cd9a747a0c441cc452537cdd9c92943b07282c46d7bad34985d9a70fa20b6f10",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/import-meta-is-an-ordinary-object.js",
        source_sha256: "5a8e3d8ea43bc5bb8afd0f83a840342970bf9f8c946dd880eec29bace81bef91",
        metadata: MODULE_IMPORT_META_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/not-accessible-from-direct-eval.js",
        source_sha256: "211f92e37e1c87e80e68a289b00efa9f4e8369e5bd8ecb6df799d744009b0a43",
        metadata: MODULE_IMPORT_META_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/same-object-returned.js",
        source_sha256: "0f34657774e235c23530da54eb56c87e3576e3744789ca7751965cb407f76a55",
        metadata: MODULE_IMPORT_META_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/escape-sequence-import.js",
        source_sha256: "ce1d9261067ef21fd9d83a4574afb78ae7d54d698902c319f79a57b942962ca0",
        metadata: MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/escape-sequence-meta.js",
        source_sha256: "c8fb14dd6b3c5b49959ba48b7024f7412fa70cdedb2c06c3cb9bf0d68e220a46",
        metadata: MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/goal-module-nested-function.js",
        source_sha256: "438ff4f1ed5818916721d5504ef7f625cb78edc2882225295efeccaa64b4c29a",
        metadata: MODULE_IMPORT_META_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/goal-module.js",
        source_sha256: "87cf3fe7e60f591aeb8c5a6ccbd2aeac7a5ea7c13ff82277043e3c3f8f8d3a74",
        metadata: MODULE_IMPORT_META_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-destructuring-expr.js",
        source_sha256: "2fbc3aae223fed21950a692929b59186bee54cb2b9e15c94b8ba9378800fa0ff",
        metadata: MODULE_IMPORT_META_DESTRUCTURING_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-rest-destructuring-expr.js",
        source_sha256: "14e7eca195c0daaa118a53df2e243b6a749f5f188cac30b1f0f511f1f441d101",
        metadata: MODULE_IMPORT_META_DESTRUCTURING_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-assignment-expr.js",
        source_sha256: "494bf34ff1ba093e983262dbcc7ff86fa7be1b3cd8971316caa1cebd050ca89b",
        metadata: MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-await-of-loop.js",
        source_sha256: "89da6cfbea147ef365c817974955739586a19eb9455c2f3a577ee0236acc8d51",
        metadata: MODULE_IMPORT_META_ASYNC_ITERATION_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-in-loop.js",
        source_sha256: "21ef3b3c4a135a0673129c72765de17e63e561e3916438b7f3510ec106167115",
        metadata: MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-of-loop.js",
        source_sha256: "5e3c3fee53c2a9c361754a1e4f3c7b2ff42fbaf2917003edcb49d309701aaf94",
        metadata: MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-destructuring-expr.js",
        source_sha256: "c7aa223f3adf7e0e51c0c9fa7d28c54e7faeb23877cb9b931cd36206b22fe5e4",
        metadata: MODULE_IMPORT_META_DESTRUCTURING_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-rest-destructuring-expr.js",
        source_sha256: "a37abbb17e395f114351b526c479b4acf9bd5ba13a0ca432c80979b2c6fe0333",
        metadata: MODULE_IMPORT_META_OBJECT_REST_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/expressions/import.meta/syntax/invalid-assignment-target-update-expr.js",
        source_sha256: "fb407018303e308e808afc656092825725d860dab800da05b8b2ab7fd90c4be3",
        metadata: MODULE_IMPORT_META_PARSE_SYNTAX_ERROR_METADATA,
        requests: &[],
    },
];

/// Admit only one of the pinned, dependency-free module roots above.
///
/// The coordinator and worker both call this function. An exact-path source or
/// metadata change is an audit failure, while an unlisted module is simply not
/// admitted and remains classified as unsupported by the coordinator.
pub(super) fn is_exact_dependency_free_module_test(
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = DEPENDENCY_FREE_MODULE_ADMISSIONS
        .iter()
        .chain(DECL_POSITION_MODULE_ADMISSIONS.iter())
        .chain(STATIC_NEGATIVE_MODULE_ADMISSIONS.iter())
        .find(|admission| path == Path::new(admission.path))
    else {
        return Ok(false);
    };
    let actual_sha256 = source_sha256(source)?;
    authenticate_dependency_free_module_test(path, &actual_sha256, metadata, admission)
}

fn authenticate_dependency_free_module_test(
    path: &Path,
    actual_sha256: &str,
    metadata: &Metadata,
    admission: &DependencyFreeModuleAdmission,
) -> Result<bool, String> {
    if path != Path::new(admission.path) {
        return Ok(false);
    }
    if actual_sha256 != admission.source_sha256 {
        return Err(format!(
            "dependency-free module source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            admission.path, admission.source_sha256
        ));
    }
    if !module_metadata_matches(metadata, admission.metadata) {
        return Err(format!(
            "dependency-free module metadata shape drifted for {}",
            admission.path
        ));
    }
    Ok(true)
}

/// Authenticate one of the deliberately narrow static-module execution
/// frontiers. An unlisted module remains unadmitted without touching any
/// fixture file; an exact graph root authenticates its complete recursive
/// closure before either the coordinator or worker can remove `module` from
/// the missing-host set.
pub(super) fn exact_module_test(
    suite: &Path,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<Option<ExactModuleTest>, String> {
    if is_exact_dependency_free_module_test(path, source, metadata)? {
        return Ok(Some(ExactModuleTest::DependencyFree));
    }
    if is_exact_fixture_graph_module_test(suite, path, source, metadata)? {
        return Ok(Some(ExactModuleTest::FixtureGraph));
    }
    Ok(None)
}

fn is_exact_fixture_graph_module_test(
    suite: &Path,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = exact_module_graph_admission(path) else {
        return Ok(false);
    };
    let root = module_graph_file(admission, admission.root_path).ok_or_else(|| {
        format!(
            "fixture graph admission has no root file: {}",
            admission.root_path
        )
    })?;
    authenticate_module_graph_file(path, source, metadata, root)?;
    authenticate_exact_module_graph_closure(admission, |relative| {
        read_regular_module_source(suite, relative)
    })?;
    Ok(true)
}

fn exact_module_graph_admission(root_path: &Path) -> Option<ExactModuleGraphAdmission> {
    if let Some(admission) = DEFAULT_MODULE_ROOT_ADMISSIONS
        .iter()
        .find(|admission| root_path == Path::new(admission.path))
    {
        return Some(ExactModuleGraphAdmission {
            root_path: admission.path,
            files: &DEFAULT_MODULE_FILE_ADMISSIONS,
            closure_file_count: admission.closure_file_count,
        });
    }
    if let Some(admission) = IMPORT_META_MODULE_ROOT_ADMISSIONS
        .iter()
        .find(|admission| root_path == Path::new(admission.path))
    {
        return Some(ExactModuleGraphAdmission {
            root_path: admission.path,
            files: &IMPORT_META_MODULE_FILE_ADMISSIONS,
            closure_file_count: admission.closure_file_count,
        });
    }
    if let Some(admission) = FIXTURE_GRAPH_MODULE_ADMISSIONS
        .iter()
        .find(|admission| root_path == Path::new(admission.root_path))
    {
        return Some(ExactModuleGraphAdmission {
            root_path: admission.root_path,
            files: admission.files,
            closure_file_count: admission.files.len(),
        });
    }
    NAMESPACE_MODULE_ROOT_ADMISSIONS
        .iter()
        .find(|admission| root_path == Path::new(admission.path))
        .map(|admission| ExactModuleGraphAdmission {
            root_path: admission.path,
            files: &NAMESPACE_MODULE_FILE_ADMISSIONS,
            closure_file_count: admission.closure_file_count,
        })
}

fn module_graph_file(
    admission: ExactModuleGraphAdmission,
    path: &str,
) -> Option<&'static ModuleGraphFileAdmission> {
    admission.files.iter().find(|file| file.path == path)
}

fn authenticate_exact_module_graph_closure(
    admission: ExactModuleGraphAdmission,
    mut read_source: impl FnMut(&str) -> Result<String, String>,
) -> Result<(), String> {
    let visited = reachable_module_graph_paths(admission)?;
    if visited.len() != admission.closure_file_count {
        return Err(format!(
            "fixture graph recursive closure size drifted for {}: expected {}, found {}",
            admission.root_path,
            admission.closure_file_count,
            visited.len()
        ));
    }
    for path in &visited {
        let file = module_graph_file(admission, path).ok_or_else(|| {
            format!(
                "fixture graph edge escaped the authenticated closure for {}: {path}",
                admission.root_path
            )
        })?;
        let source = read_source(path)?;
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse authenticated module metadata for {path}: {error}"))?;
        authenticate_module_graph_file(Path::new(path), &source, &metadata, file)?;
    }
    Ok(())
}

fn reachable_module_graph_paths(
    admission: ExactModuleGraphAdmission,
) -> Result<BTreeSet<&'static str>, String> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![admission.root_path];
    while let Some(path) = pending.pop() {
        if !visited.insert(path) {
            continue;
        }
        let file = module_graph_file(admission, path).ok_or_else(|| {
            format!(
                "fixture graph edge escaped the authenticated closure for {}: {path}",
                admission.root_path
            )
        })?;
        for request in file.requests.iter().rev() {
            if module_graph_file(admission, request.normalized_path).is_none() {
                return Err(format!(
                    "fixture graph request escaped the authenticated closure for {}: {} -> {}",
                    admission.root_path, request.specifier, request.normalized_path
                ));
            }
            pending.push(request.normalized_path);
        }
    }
    Ok(visited)
}

#[cfg(test)]
fn authenticate_fixture_graph_closure(
    admission: &FixtureGraphModuleAdmission,
    read_source: impl FnMut(&str) -> Result<String, String>,
) -> Result<(), String> {
    authenticate_exact_module_graph_closure(
        ExactModuleGraphAdmission {
            root_path: admission.root_path,
            files: admission.files,
            closure_file_count: admission.files.len(),
        },
        read_source,
    )
}

fn authenticate_module_graph_file(
    path: &Path,
    source: &str,
    metadata: &Metadata,
    file: &ModuleGraphFileAdmission,
) -> Result<(), String> {
    let actual_sha256 = source_sha256(source)?;
    authenticate_module_graph_file_digest(path, &actual_sha256, metadata, file)
}

fn authenticate_module_graph_file_digest(
    path: &Path,
    actual_sha256: &str,
    metadata: &Metadata,
    file: &ModuleGraphFileAdmission,
) -> Result<(), String> {
    if path != Path::new(file.path) {
        return Err(format!(
            "fixture graph file path drifted: expected {}, found {}",
            file.path,
            path.display()
        ));
    }
    if actual_sha256 != file.source_sha256 {
        return Err(format!(
            "fixture graph module source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            file.path, file.source_sha256
        ));
    }
    if !module_metadata_matches(metadata, file.metadata) {
        return Err(format!(
            "fixture graph module metadata shape drifted for {}",
            file.path
        ));
    }
    Ok(())
}

fn read_regular_module_source(suite: &Path, relative: &str) -> Result<String, String> {
    let path = suite.join(relative);
    let file_type = fs::symlink_metadata(&path)
        .map_err(|error| format!("stat authenticated module {}: {error}", path.display()))?
        .file_type();
    if !file_type.is_file() || file_type.is_symlink() {
        return Err(format!(
            "authenticated module is not a regular non-symlink file: {}",
            path.display()
        ));
    }
    fs::read_to_string(&path)
        .map_err(|error| format!("read authenticated module {}: {error}", path.display()))
}

/// Normalize only a source-authenticated request edge from one admitted graph.
/// This deliberately refuses generic path joining, bare names, and requests
/// from an unlisted graph member.
pub(super) fn normalize_exact_module_request(
    root_path: &Path,
    base_name: &str,
    specifier: &str,
) -> Result<String, String> {
    let admission = exact_module_graph_admission(root_path).ok_or_else(|| {
        format!(
            "module loader rejected unaudited root: {}",
            root_path.display()
        )
    })?;
    let reachable = reachable_module_graph_paths(admission)?;
    if !reachable.contains(base_name) {
        return Err(format!(
            "module loader rejected unaudited base module: {base_name}"
        ));
    }
    let base = module_graph_file(admission, base_name)
        .ok_or_else(|| format!("module loader rejected unaudited base module: {base_name}"))?;
    let request = base
        .requests
        .iter()
        .find(|request| request.specifier == specifier)
        .ok_or_else(|| {
            format!("module loader rejected unaudited request from {base_name}: {specifier}")
        })?;
    Ok(request.normalized_path.to_owned())
}

/// Load one exact fixture from a previously authenticated graph. The source
/// and metadata are checked again at the loader boundary to close the gap
/// between coordinator admission and worker resolution.
pub(super) fn load_exact_module_fixture(
    suite: &Path,
    root_path: &Path,
    normalized_name: &str,
) -> Result<String, String> {
    let admission = exact_module_graph_admission(root_path).ok_or_else(|| {
        format!(
            "module loader rejected unaudited root: {}",
            root_path.display()
        )
    })?;
    let reachable = reachable_module_graph_paths(admission)?;
    if !reachable.contains(normalized_name) {
        return Err(format!(
            "module loader rejected unaudited fixture: {normalized_name}"
        ));
    }
    let file = module_graph_file(admission, normalized_name)
        .filter(|file| file.path != admission.root_path)
        .ok_or_else(|| format!("module loader rejected unaudited fixture: {normalized_name}"))?;
    let source = read_regular_module_source(suite, file.path)?;
    let metadata = parse_metadata(&source).map_err(|error| {
        format!(
            "parse authenticated module metadata for {}: {error}",
            file.path
        )
    })?;
    authenticate_module_graph_file(Path::new(file.path), &source, &metadata, file)?;
    Ok(source)
}

fn module_metadata_matches(metadata: &Metadata, contract: ModuleMetadataContract) -> bool {
    metadata
        .includes
        .iter()
        .map(String::as_str)
        .eq(contract.includes.iter().copied())
        && metadata
            .flags
            .iter()
            .map(String::as_str)
            .eq(contract.flags.iter().copied())
        && metadata
            .features
            .iter()
            .map(String::as_str)
            .eq(contract.features.iter().copied())
        && match (&metadata.negative, contract.negative) {
            (None, None) => true,
            (Some(actual), Some(expected)) => {
                actual.phase.as_deref() == Some(expected.phase)
                    && actual.error_type.as_deref() == Some(expected.error_type)
            }
            _ => false,
        }
}

struct AgentHostAdmission {
    path: &'static str,
    source_sha256: &'static str,
    features: &'static [&'static str],
    cohort: &'static str,
}

const AGENT_HOST_ADMISSIONS: [AgentHostAdmission; 59] = [
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/bigint/notify-all-on-loc.js",
        source_sha256: "442a9e3af420e81107defd515e5bfe539a7a5a133e61797fad9a640e93439b3d",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/count-defaults-to-infinity-missing.js",
        source_sha256: "5bc3aee123dafa5dd70ff92a8c73385880a26118e5d110f79d809707225f6a6b",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/count-defaults-to-infinity-undefined.js",
        source_sha256: "57afbd3a2f85800ee919c038809d605511102f6ee99504f5b833f97cf75c7efb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/negative-count.js",
        source_sha256: "fe734b6972c67082995e6140e781449198828b79d0ddb24a51a204b5afd6390e",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-all-on-loc.js",
        source_sha256: "f2f60a1f70c6f6c47d28ad602418889b81586b4d4c1f06e8c09e063c4e510844",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-all.js",
        source_sha256: "0a68a903a51def1d8869c2c93fb7e3640bf6389f148482e5a4cb8bc42e7926d9",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-in-order-one-time.js",
        source_sha256: "9cdc624fc8932d14b137b5daf34bf27efedce16fb53a0f4ef94fcdd0f26af989",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-in-order.js",
        source_sha256: "9cdc624fc8932d14b137b5daf34bf27efedce16fb53a0f4ef94fcdd0f26af989",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-nan.js",
        source_sha256: "9d022e8e59572cbcd5dc672b3249b9c67407e9007c7e56f4156b4aea2e4857c5",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-one.js",
        source_sha256: "3364d4844004ba73efe5036da4fd0cafa1bab5218885946e5b704bd06082dd61",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-renotify-noop.js",
        source_sha256: "e69b68f7240ff876c28b5ed4130a54830eaf6e02e665db6c87e0b7c1a1cbafdb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-two.js",
        source_sha256: "03309fe924420caf6fc40817dce1095895470233268502687a2168a27769d9ab",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-with-no-agents-waiting.js",
        source_sha256: "c4f49f9a52daab30e695cea6d8fe400a7ebd38dc41daef6843b763d1006ba718",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-with-no-matching-agents-waiting.js",
        source_sha256: "85e1c3a5897d64f38b6f271b714cb025ed237267d47ebf9ab332a19b03e1a382",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-zero.js",
        source_sha256: "57018cbe3c726eeecbbce24f9b40a3ab5f845372030731b0255adbbbda27c80f",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/undefined-index-defaults-to-zero.js",
        source_sha256: "9235c0501b3f81cb4b7079ee73e52de6f39f987467d90b79b9c47bb44baf6550",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/false-for-timeout-agent.js",
        source_sha256: "30818849f231757c0fce413f31fa235c63236f9268eab982ce58078d427fade1",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/nan-for-timeout.js",
        source_sha256: "7109bf013ce44e8e36d88ce1eda639b0f844fee89c04f31cf8356475b6b89021",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/negative-timeout-agent.js",
        source_sha256: "098159fb9b6c3619ee5eaf445333bf5b20088fc46e9227c8de383bfd3550b014",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-no-operation.js",
        source_sha256: "9002df4475d2b76914f49e2c431e77a3396a1cc114f0715078fbdf8eb11346ee",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-add.js",
        source_sha256: "1a661c6660fbb3a33fbc097ff1af549ec5995e69758871b4851f88d36531a676",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-and.js",
        source_sha256: "8e097032ed544fcbf3c0290d4324dfbb3fa782c8669b3299b74b580b4af9223c",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-compareExchange.js",
        source_sha256: "0900f28d7cedcd006904fea08be5953415c83af9ea579ac1c31b12efb7ae612a",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-exchange.js",
        source_sha256: "27e9693ceb73db3d177d57899cf5240251af31617cb31d7ca8c21aa3848130f3",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-or.js",
        source_sha256: "b31f24fa0de4383b7a85d504629a181cba7d8400707664fc084c90c7ca29d57c",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-store.js",
        source_sha256: "ace9fe8ca799b7c9898a263f10df37beaf2ca97cc1aaaed5382aaeedce275989",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-sub.js",
        source_sha256: "5a1a1b1eff5407f32f2195f4f5a45f610c51dfae67d7e4fcf5230d600957c546",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-xor.js",
        source_sha256: "c4c1bf8012da172bdc5114e995869a5a82a2b01d88fd6a53a39d7fffc5445e3e",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/value-not-equal.js",
        source_sha256: "6ac2ae7a18c6081df18371c6dab12bb82430f37e92ec6a6c3ff9ff5ce59df700",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/waiterlist-block-indexedposition-wake.js",
        source_sha256: "b02f89aa4a6fc7cc8e6f63c7761b95a2cfe08bd2bdb2483e6f9c4c0462975e95",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/waiterlist-order-of-operations-is-fifo.js",
        source_sha256: "bfa8cc8764efee31ea7bda7f25755853e5fbf3b109ddc72650e65c53058b3f88",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/was-woken-before-timeout.js",
        source_sha256: "f7af53430000b4c57d0e50314cf9f1a5c68f3f9f40f9d0da26fdeb40651cd11e",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/false-for-timeout-agent.js",
        source_sha256: "1f155c405b5b137c902e5e385a5a39a858444ad63941bdde5ca6762844e978a2",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/good-views.js",
        source_sha256: "7ab45f324e0f668a9d9f3df03c866b0ac32276eb1dfb649d1e5783a88f70bb21",
        features: &["Atomics"],
        cohort: "Test262 agent Stage A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/nan-for-timeout.js",
        source_sha256: "efaa0c6981a9a485a0dd40b145fd071f3667d4d7bef28a5c844a22f9bcd1c1d2",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/negative-timeout-agent.js",
        source_sha256: "8d2236937f9a3d792cfda706d7d7703642c21bbda26729ca29421d89cb3865eb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-no-operation.js",
        source_sha256: "7436557067aa3940e9882a53387257a60ef034d6173c10335ad8f5415d15ceb9",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-add.js",
        source_sha256: "672834f107ba1c574a19323aca5284b45dbf6db0384892e9388629127ca7015c",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-and.js",
        source_sha256: "f4b63fb173a054c591a38d36bdf2d74181c1596571b3f2857d501b4c3cda1469",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-compareExchange.js",
        source_sha256: "51129ba0e54af3cea300b23b85a631f2186dab1e264b7b05edb214e1d4048eb4",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-exchange.js",
        source_sha256: "03068ee53c5a70deb59271de3311190e9524c0dffa1a081a17593103a1e1c9c9",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-or.js",
        source_sha256: "6e15a69b550977979fe2bed9a60d667a0e194f2a7ade50bc92f1e82c2fe3a086",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-store.js",
        source_sha256: "f018200f54d42e169cda405f92f99f017abae44bb2aae18319633d857d3d7171",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-sub.js",
        source_sha256: "3f77bf071ef009ebb098d3a43e2670b847bea3954177df1751252dcd07f1c5e5",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-xor.js",
        source_sha256: "be9af683186fd217591b733ca6cb685db3b091b28331cdfd609d0cb756fb9e04",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/null-for-timeout-agent.js",
        source_sha256: "407d2a0a8bf72382dfeb22b711cce26ea562a8b2da1c79e941c50315e78f7a30",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/object-for-timeout-agent.js",
        source_sha256: "c7ecd98803298b5fbc82f6f68d16bce6f3246800a9ef79b526ae55be06d41d0f",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/poisoned-object-for-timeout-throws-agent.js",
        source_sha256: "2780f367fba1a8090ac059185fc8dd3d7f92da10dea9261ff9eb00845ef3c266",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/symbol-for-index-throws-agent.js",
        source_sha256: "b255a1f336e1fa3de54eff1a885a5b8c52d1d307ca2d21e73e5a8c5cbb472c1f",
        features: &[
            "Atomics",
            "SharedArrayBuffer",
            "Symbol",
            "Symbol.toPrimitive",
            "TypedArray",
        ],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/symbol-for-timeout-throws-agent.js",
        source_sha256: "6d37e6f2f0db2518c31b41e08aa2479e07d425ed1510a5170a30277b8698c172",
        features: &[
            "Atomics",
            "SharedArrayBuffer",
            "Symbol",
            "Symbol.toPrimitive",
            "TypedArray",
        ],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/symbol-for-value-throws-agent.js",
        source_sha256: "7176e285cd33104da37b6cc70a2f5e83a9165da02092ad15062088ed7d83b5de",
        features: &[
            "Atomics",
            "SharedArrayBuffer",
            "Symbol",
            "Symbol.toPrimitive",
            "TypedArray",
        ],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/true-for-timeout-agent.js",
        source_sha256: "742792a79f511dd8581771d134c8355bd39d7eb90b70884e6ef5e3a810680cec",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/undefined-for-timeout.js",
        source_sha256: "0dd3f74bb8ae3b06012e1b1b047fcd1e499943e63829214da4c16fc49df5d589",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/undefined-index-defaults-to-zero.js",
        source_sha256: "c0b85d26b9e50ee0c309d55d90f4a30a06ebd7139a57197e55b7c0ecec9a95fb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/value-not-equal.js",
        source_sha256: "24a38831488f8794736387ab7cafc0528fc9fd9f2276b49a5e04d77f5ef0e4a7",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/wait-index-value-not-equal.js",
        source_sha256: "0c2103b7079f54cfbe0c57ccbaef6644bab370409dad2f32e6b0c3e9577dfa08",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/waiterlist-block-indexedposition-wake.js",
        source_sha256: "87e398dbfc8e4022331380d67325a2da98dea734dfed11158ab0e34e0f417ab3",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/waiterlist-order-of-operations-is-fifo.js",
        source_sha256: "6503e1b20e4c55d661c165020ee7b83a3cd35326fed0211358df41b67b2adda1",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/was-woken-before-timeout.js",
        source_sha256: "d97f474e3fe55e36d6475ef88653ce4e4b203e638d2f28f5547f9c2b30784d2a",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
];

/// Admit only source- and metadata-audited `$262.agent` tests.
///
/// The exact path check prevents a profile entry from broadening the host
/// surface. The source hash and complete metadata shape prevent an in-place
/// Test262 update from silently inheriting an earlier admission.
pub(super) fn is_exact_agent_host_test(
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = AGENT_HOST_ADMISSIONS
        .iter()
        .find(|admission| path == Path::new(admission.path))
    else {
        return Ok(false);
    };
    let actual_sha256 = source_sha256(source)?;
    if actual_sha256 != admission.source_sha256 {
        return Err(format!(
            "{} source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            admission.cohort, admission.path, admission.source_sha256
        ));
    }
    if !agent_host_metadata_matches(metadata, admission) {
        return Err(format!(
            "{} metadata shape drifted for {}",
            admission.cohort, admission.path
        ));
    }
    Ok(true)
}

fn agent_host_metadata_matches(metadata: &Metadata, admission: &AgentHostAdmission) -> bool {
    metadata.includes == ["atomicsHelper.js"]
        && metadata.flags.is_empty()
        && metadata
            .features
            .iter()
            .map(String::as_str)
            .eq(admission.features.iter().copied())
        && metadata.negative.is_none()
}

/// Return conservative, stable IDs for Test262 execution capabilities which
/// the current runner cannot provide.
///
/// Metadata is authoritative for declared execution modes. Includes and `$262`
/// source tokens are hints for host hooks: JavaScript can replace the writable
/// `$262` global, so the execution layer must still retain dynamic provenance
/// before treating one of those hook hints as the cause of a result.
pub(super) fn missing_host_capability_hints(
    path: &Path,
    source: &str,
    metadata: &Metadata,
    allow_async: bool,
) -> Vec<String> {
    let mut missing = BTreeSet::new();
    // Host-hook discovery is intentionally fail-closed: do not apply the
    // approximate RegExp lexical goal used by the scoped async audit, because
    // mistaking division for a literal could hide a real `$262` access.
    let tokens = source_tokens(source, false);

    if metadata.is_module() {
        missing.insert("module".to_owned());
    }
    if metadata.is_async() && !allow_async {
        missing.insert("async".to_owned());
    }
    if metadata.flags.contains("CanBlockIsFalse") {
        missing.insert("can-block:false".to_owned());
    }

    // These feature names are explicit Test262 host requirements at the
    // pinned suite revision. `cross-realm` is deliberately not mapped here:
    // that feature is neither necessary nor sufficient evidence that the test
    // actually calls `$262.createRealm`.
    if metadata
        .features
        .iter()
        .any(|feature| feature == "host-gc-required")
    {
        missing.insert("gc".to_owned());
    }
    if metadata
        .features
        .iter()
        .any(|feature| feature == "IsHTMLDDA")
    {
        missing.insert("is-html-dda".to_owned());
    }

    let shadows_host_262 = is_detach_helper_shadow_test(path, &tokens);

    // atomicsHelper.js immediately consumes `$262.agent`. The detach helper
    // normally consumes `$262.detachArrayBuffer` when the test calls it, except
    // for the harness self-test which intentionally installs its own `$262`.
    for include in &metadata.includes {
        match include.as_str() {
            "atomicsHelper.js" => {
                missing.insert("agent".to_owned());
            }
            "detachArrayBuffer.js" if !shadows_host_262 => {
                missing.insert("detach-array-buffer".to_owned());
            }
            // The QuickJS patch makes this an optional fast path with a
            // JavaScript fallback. Absence is not a host requirement.
            "regExpUtils.js" => {}
            _ => {}
        }
    }

    for hook in member_names(&tokens) {
        let capability = match hook {
            "agent" => Some("agent"),
            "createRealm" => Some("create-realm"),
            "evalScript" => Some("eval-script"),
            "detachArrayBuffer" => Some("detach-array-buffer"),
            "IsHTMLDDA" => Some("is-html-dda"),
            "gc" => Some("gc"),
            "AbstractModuleSource" => Some("abstract-module-source"),
            "global" => Some("global"),
            // codePointRange is a QuickJS-only optional optimization used by
            // patched harness code and must remain absent when unsupported so
            // `typeof` can select the fallback.
            "codePointRange" => None,
            unknown => {
                missing.insert(format!("unknown:$262.{unknown}"));
                None
            }
        };
        if let Some(capability) = capability {
            missing.insert(capability.to_owned());
        }
    }

    missing.into_iter().collect()
}

/// Return pinned source-audited feature requirements omitted by Test262
/// metadata or deliberately staged behind an explicit host-admission tag.
///
/// `createRealm` and `evalScript` have no standard Test262 feature tag, so
/// synthetic tags keep newly implemented worker hooks from silently changing
/// the global conformance vector before their admission gates. The
/// SpiderMonkey Atomics staging tests additionally omit feature metadata. The
/// cross-compartment test constructs a foreign `SharedArrayBuffer`, while the
/// detached-buffer test exercises non-shared `Atomics` operations. Keep these
/// path overrides exact and fail closed if their audited source changes.
pub(super) fn supplemental_feature_hints(path: &Path, source: &str) -> Result<Vec<String>, String> {
    const ATOMICS_CROSS_REALM: &str = "test/staging/sm/Atomics/cross-compartment.js";
    const ATOMICS_CROSS_REALM_SHA256: &str =
        "8b6770fe9be68c0deed01fdc484da4b80737f7068ef1c823dae3ea30de885f56";
    const ATOMICS_DETACHED_BUFFERS: &str = "test/staging/sm/Atomics/detached-buffers.js";
    const ATOMICS_DETACHED_BUFFERS_SHA256: &str =
        "c7813d0121f03dc3c97e088afccca800220e494d27ae0b75d89464f41598ee12";

    let tokens = source_tokens(source, false);
    let members = member_names(&tokens);
    let mut hints = BTreeSet::new();
    if members.contains(&"createRealm") {
        hints.insert("host-create-realm-required".to_owned());
    }
    if members.contains(&"evalScript") {
        hints.insert("host-eval-script-required".to_owned());
    }

    insert_atomics_cross_realm_feature_hints(
        &mut hints,
        path,
        source,
        &tokens,
        ATOMICS_CROSS_REALM,
        ATOMICS_CROSS_REALM_SHA256,
    )?;

    insert_exact_source_feature_hint(
        &mut hints,
        path,
        source,
        ATOMICS_DETACHED_BUFFERS,
        ATOMICS_DETACHED_BUFFERS_SHA256,
        "Atomics",
    )?;

    Ok(hints.into_iter().collect())
}

fn insert_atomics_cross_realm_feature_hints(
    hints: &mut BTreeSet<String>,
    path: &Path,
    source: &str,
    tokens: &[SourceToken<'_>],
    expected_path: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    if !verify_exact_source_sha256(path, source, expected_path, expected_sha256)? {
        return Ok(());
    }
    let has_identifier = |wanted| {
        tokens
            .iter()
            .any(|token| matches!(token, SourceToken::Identifier(name) if *name == wanted))
    };
    if !hints.contains("host-create-realm-required")
        || !has_identifier("Atomics")
        || !has_identifier("SharedArrayBuffer")
    {
        return Err(format!(
            "supplemental feature source shape drifted for {expected_path}"
        ));
    }
    hints.insert("Atomics".to_owned());
    hints.insert("SharedArrayBuffer".to_owned());
    Ok(())
}

fn insert_exact_source_feature_hint(
    hints: &mut BTreeSet<String>,
    path: &Path,
    source: &str,
    expected_path: &str,
    expected_sha256: &str,
    feature: &str,
) -> Result<(), String> {
    if !verify_exact_source_sha256(path, source, expected_path, expected_sha256)? {
        return Ok(());
    }
    hints.insert(feature.to_owned());
    Ok(())
}

fn verify_exact_source_sha256(
    path: &Path,
    source: &str,
    expected_path: &str,
    expected_sha256: &str,
) -> Result<bool, String> {
    if path != Path::new(expected_path) {
        return Ok(false);
    }
    let actual_sha256 = source_sha256(source)?;
    if actual_sha256 != expected_sha256 {
        return Err(format!(
            "supplemental feature audit drifted for {expected_path}: expected source SHA-256 \
             {expected_sha256}, found {actual_sha256}"
        ));
    }
    Ok(true)
}

fn source_sha256(source: &str) -> Result<String, String> {
    let commands: [(&str, &[&str]); 2] = [("sha256sum", &[]), ("shasum", &["-a", "256"])];
    let mut unavailable = Vec::new();
    for (program, arguments) in commands {
        let mut child = match Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                unavailable.push(program);
                continue;
            }
            Err(error) => return Err(format!("hash Test262 source with {program}: {error}")),
        };
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| format!("hash Test262 source with {program}: stdin unavailable"))?;
            stdin
                .write_all(source.as_bytes())
                .map_err(|error| format!("hash Test262 source with {program}: {error}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("hash Test262 source with {program}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "hash Test262 source with {program}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        return Ok(String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned());
    }
    Err(format!(
        "cannot hash Test262 source: commands are unavailable: {}",
        unavailable.join(", ")
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceToken<'source> {
    Identifier(&'source str),
    Dot,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Arrow,
    LineTerminator,
    Literal,
    Other(u8),
}

fn member_names<'source>(tokens: &[SourceToken<'source>]) -> Vec<&'source str> {
    significant_tokens(tokens)
        .windows(3)
        .filter_map(|window| match window {
            [
                SourceToken::Identifier("$262"),
                SourceToken::Dot,
                SourceToken::Identifier(name),
            ] => Some(*name),
            _ => None,
        })
        .collect()
}

fn is_detach_helper_shadow_test(path: &Path, tokens: &[SourceToken<'_>]) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.ends_with("test/harness/detachArrayBuffer-host-detachArrayBuffer.js")
        && significant_tokens(tokens).windows(2).any(|window| {
            matches!(
                window,
                [
                    SourceToken::Identifier("var" | "let" | "const"),
                    SourceToken::Identifier("$262")
                ]
            )
        })
}

fn significant_tokens<'source>(tokens: &[SourceToken<'source>]) -> Vec<SourceToken<'source>> {
    tokens
        .iter()
        .copied()
        .filter(|token| !matches!(token, SourceToken::LineTerminator))
        .collect()
}

/// Return whether one test in the pinned generator/destructuring admission
/// cohort contains async function or async-arrow grammar which its
/// non-exhaustive feature metadata does not declare.
///
/// This is deliberately not a general JavaScript parser. The feature check
/// keeps the lexical audit inside the checksum-bound cohort whose synchronous
/// complement is independently run by the R3t gate. The coordinator uses it
/// only as the final admission guard after every authoritative classification
/// has accepted the test.
pub(super) fn generator_destructuring_source_needs_async_guard(
    source: &str,
    metadata: &Metadata,
) -> bool {
    metadata
        .features
        .iter()
        .any(|feature| matches!(feature.as_str(), "generators" | "destructuring-binding"))
        && contains_async_function_or_arrow_syntax(&source_tokens(source, true))
}

fn contains_async_function_or_arrow_syntax(tokens: &[SourceToken<'_>]) -> bool {
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, SourceToken::Identifier("async")) {
            continue;
        }

        let Some((head_index, false)) = next_significant_token(tokens, index + 1) else {
            continue;
        };
        match tokens[head_index] {
            SourceToken::Identifier("function") => return true,
            SourceToken::Identifier(_) => {
                if let Some((next_index, crossed_line_terminator)) =
                    next_significant_token(tokens, head_index + 1)
                {
                    if matches!(tokens[next_index], SourceToken::Arrow) && !crossed_line_terminator
                    {
                        return true;
                    }
                }
            }
            SourceToken::LeftParen => {
                let mut depth = 1usize;
                let mut cursor = head_index + 1;
                while cursor < tokens.len() {
                    match tokens[cursor] {
                        SourceToken::LeftParen => depth += 1,
                        SourceToken::RightParen => {
                            depth -= 1;
                            if depth == 0 {
                                if matches!(
                                    next_significant_token(tokens, cursor + 1),
                                    Some((arrow_index, false))
                                        if matches!(tokens[arrow_index], SourceToken::Arrow)
                                ) {
                                    return true;
                                }
                                break;
                            }
                        }
                        _ => {}
                    }
                    cursor += 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Return the next code token and whether a line terminator occurred before it.
fn next_significant_token(tokens: &[SourceToken<'_>], mut index: usize) -> Option<(usize, bool)> {
    let mut crossed_line_terminator = false;
    while index < tokens.len() {
        if matches!(tokens[index], SourceToken::LineTerminator) {
            crossed_line_terminator = true;
            index += 1;
        } else {
            return Some((index, crossed_line_terminator));
        }
    }
    None
}

fn source_tokens(source: &str, skip_regexp_literals: bool) -> Vec<SourceToken<'_>> {
    let mut tokens = Vec::new();
    let mut index = 0;
    scan_code(source, &mut index, None, skip_regexp_literals, &mut tokens);
    tokens
}

/// Tokenize only the small lexical surface needed for `$262 . hook` hints and
/// async callable classification. Full parsing is intentionally avoided
/// because unsupported grammar is one of the things the Test262 runner
/// measures.
fn scan_code<'source>(
    source: &'source str,
    index: &mut usize,
    mut template_brace_depth: Option<usize>,
    skip_regexp_literals: bool,
    tokens: &mut Vec<SourceToken<'source>>,
) {
    let bytes = source.as_bytes();
    while *index < bytes.len() {
        if let Some(length) = line_terminator_length(bytes, *index) {
            push_line_terminator(tokens);
            *index += length;
            continue;
        }

        let byte = bytes[*index];
        let next = bytes.get(*index + 1).copied();
        match (byte, next) {
            (b'/', Some(b'/')) => skip_line_comment(bytes, index),
            (b'/', Some(b'*')) => {
                if skip_block_comment(bytes, index) {
                    push_line_terminator(tokens);
                }
            }
            (b'/', _)
                if skip_regexp_literals
                    && regexp_literal_allowed(tokens)
                    && skip_regexp_literal(bytes, index) =>
            {
                tokens.push(SourceToken::Literal);
            }
            (b'\'' | b'"', _) => {
                skip_quoted_string(bytes, index, byte);
                tokens.push(SourceToken::Literal);
            }
            (b'`', _) => scan_template(source, index, skip_regexp_literals, tokens),
            (b'{', _) if template_brace_depth.is_some() => {
                template_brace_depth = template_brace_depth.map(|depth| depth + 1);
                tokens.push(SourceToken::LeftBrace);
                *index += 1;
            }
            (b'}', _) if template_brace_depth.is_some() => {
                let depth = template_brace_depth.expect("template depth was checked");
                *index += 1;
                if depth == 1 {
                    return;
                }
                template_brace_depth = Some(depth - 1);
                tokens.push(SourceToken::RightBrace);
            }
            (b'.', _) => {
                tokens.push(SourceToken::Dot);
                *index += 1;
            }
            (b'(', _) => {
                tokens.push(SourceToken::LeftParen);
                *index += 1;
            }
            (b')', _) => {
                tokens.push(SourceToken::RightParen);
                *index += 1;
            }
            (b'[', _) => {
                tokens.push(SourceToken::LeftBracket);
                *index += 1;
            }
            (b']', _) => {
                tokens.push(SourceToken::RightBracket);
                *index += 1;
            }
            (b'{', _) => {
                tokens.push(SourceToken::LeftBrace);
                *index += 1;
            }
            (b'}', _) => {
                tokens.push(SourceToken::RightBrace);
                *index += 1;
            }
            (b'=', Some(b'>')) => {
                tokens.push(SourceToken::Arrow);
                *index += 2;
            }
            (byte, _) if is_ascii_identifier_start(byte) => {
                let start = *index;
                *index += 1;
                while *index < bytes.len() && is_ascii_identifier_continue(bytes[*index]) {
                    *index += 1;
                }
                tokens.push(SourceToken::Identifier(&source[start..*index]));
            }
            (byte, _) if byte.is_ascii_digit() => {
                skip_number(bytes, index);
                tokens.push(SourceToken::Literal);
            }
            (byte, _) if byte.is_ascii_whitespace() => *index += 1,
            _ => {
                tokens.push(SourceToken::Other(byte));
                *index += 1;
            }
        }
    }
}

fn scan_template<'source>(
    source: &'source str,
    index: &mut usize,
    skip_regexp_literals: bool,
    tokens: &mut Vec<SourceToken<'source>>,
) {
    let bytes = source.as_bytes();
    // Keep code tokens on either side of a template literal separate while
    // still scanning `${ ... }` substitutions using the code lexical goal.
    tokens.push(SourceToken::Literal);
    *index += 1;
    while *index < bytes.len() {
        match (bytes[*index], bytes.get(*index + 1).copied()) {
            (b'\\', _) => {
                *index += 1;
                if *index < bytes.len() {
                    *index += 1;
                }
            }
            (b'`', _) => {
                *index += 1;
                return;
            }
            (b'$', Some(b'{')) => {
                *index += 2;
                tokens.push(SourceToken::Other(b'{'));
                scan_code(source, index, Some(1), skip_regexp_literals, tokens);
                tokens.push(SourceToken::Literal);
            }
            _ => *index += 1,
        }
    }
}

fn skip_line_comment(bytes: &[u8], index: &mut usize) {
    *index += 2;
    while *index < bytes.len() && line_terminator_length(bytes, *index).is_none() {
        *index += 1;
    }
}

fn skip_block_comment(bytes: &[u8], index: &mut usize) -> bool {
    let mut contained_line_terminator = false;
    *index += 2;
    while *index < bytes.len() {
        if bytes[*index] == b'*' && bytes.get(*index + 1) == Some(&b'/') {
            *index += 2;
            return contained_line_terminator;
        }
        if let Some(length) = line_terminator_length(bytes, *index) {
            contained_line_terminator = true;
            *index += length;
        } else {
            *index += 1;
        }
    }
    contained_line_terminator
}

fn skip_quoted_string(bytes: &[u8], index: &mut usize, quote: u8) {
    *index += 1;
    while *index < bytes.len() {
        match bytes[*index] {
            b'\\' => {
                *index += 1;
                if *index < bytes.len() {
                    *index += 1;
                }
            }
            byte if byte == quote => {
                *index += 1;
                return;
            }
            _ => *index += 1,
        }
    }
}

fn skip_number(bytes: &[u8], index: &mut usize) {
    *index += 1;
    while *index < bytes.len()
        && (bytes[*index].is_ascii_alphanumeric()
            || matches!(bytes[*index], b'_' | b'.')
            || ((bytes[*index] == b'+' || bytes[*index] == b'-')
                && matches!(bytes.get(*index - 1), Some(b'e' | b'E' | b'p' | b'P'))))
    {
        *index += 1;
    }
}

fn regexp_literal_allowed(tokens: &[SourceToken<'_>]) -> bool {
    let previous = tokens
        .iter()
        .rev()
        .find(|token| !matches!(token, SourceToken::LineTerminator));
    match previous {
        None => true,
        Some(SourceToken::Identifier(keyword)) => matches!(
            *keyword,
            "await"
                | "case"
                | "delete"
                | "do"
                | "else"
                | "in"
                | "instanceof"
                | "new"
                | "of"
                | "return"
                | "throw"
                | "typeof"
                | "void"
                | "yield"
        ),
        Some(
            SourceToken::Dot
            | SourceToken::RightParen
            | SourceToken::RightBracket
            | SourceToken::RightBrace
            | SourceToken::Literal,
        ) => false,
        Some(
            SourceToken::LeftParen
            | SourceToken::LeftBracket
            | SourceToken::LeftBrace
            | SourceToken::Arrow
            | SourceToken::Other(_)
            | SourceToken::LineTerminator,
        ) => true,
    }
}

fn skip_regexp_literal(bytes: &[u8], index: &mut usize) -> bool {
    let mut cursor = *index + 1;
    let mut in_character_class = false;
    while cursor < bytes.len() {
        if line_terminator_length(bytes, cursor).is_some() {
            return false;
        }
        match bytes[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < bytes.len() {
                    cursor += 1;
                }
            }
            b'[' if !in_character_class => {
                in_character_class = true;
                cursor += 1;
            }
            b']' if in_character_class => {
                in_character_class = false;
                cursor += 1;
            }
            b'/' if !in_character_class => {
                cursor += 1;
                while cursor < bytes.len() && is_ascii_identifier_continue(bytes[cursor]) {
                    cursor += 1;
                }
                *index = cursor;
                return true;
            }
            _ => cursor += 1,
        }
    }
    false
}

fn push_line_terminator(tokens: &mut Vec<SourceToken<'_>>) {
    if !matches!(tokens.last(), Some(SourceToken::LineTerminator)) {
        tokens.push(SourceToken::LineTerminator);
    }
}

fn line_terminator_length(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index..) {
        Some([b'\r', b'\n', ..]) => Some(2),
        Some([b'\n' | b'\r', ..]) => Some(1),
        Some([0xe2, 0x80, 0xa8 | 0xa9, ..]) => Some(3),
        _ => None,
    }
}

const fn is_ascii_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

const fn is_ascii_identifier_continue(byte: u8) -> bool {
    is_ascii_identifier_start(byte) || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        AGENT_HOST_ADMISSIONS, DECL_POSITION_MODULE_ADMISSIONS, DEFAULT_MODULE_FILE_ADMISSIONS,
        DEFAULT_MODULE_ROOT_ADMISSIONS, DEPENDENCY_FREE_MODULE_ADMISSIONS,
        ExactModuleGraphAdmission, ExactModuleTest, FIXTURE_GRAPH_MODULE_ADMISSIONS,
        FixtureGraphModuleAdmission, HostCapabilities, IMPORT_META_MODULE_FILE_ADMISSIONS,
        IMPORT_META_MODULE_ROOT_ADMISSIONS, MODULE_FIXTURE_METADATA, MODULE_METADATA,
        ModuleGraphFileAdmission, ModuleMetadataContract, ModuleRequestAdmission,
        NAMESPACE_MODULE_FILE_ADMISSIONS, NAMESPACE_MODULE_ROOT_ADMISSIONS,
        STATIC_NEGATIVE_MODULE_ADMISSIONS, agent_host_metadata_matches,
        authenticate_dependency_free_module_test, authenticate_exact_module_graph_closure,
        authenticate_fixture_graph_closure, authenticate_module_graph_file,
        authenticate_module_graph_file_digest, exact_module_graph_admission, exact_module_test,
        generator_destructuring_source_needs_async_guard, insert_atomics_cross_realm_feature_hints,
        insert_exact_source_feature_hint, is_exact_agent_host_test,
        is_exact_dependency_free_module_test, missing_host_capability_hints,
        module_metadata_matches, normalize_exact_module_request, reachable_module_graph_paths,
        source_sha256, source_tokens, supplemental_feature_hints,
    };
    use crate::metadata::{Metadata, NegativeExpectation, parse_metadata};

    const DEFAULT_MODULE_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a.txt"
    ));
    const DEFAULT_MODULE_SOURCES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-sources.txt"
    ));
    const DEFAULT_MODULE_EDGES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-edges.tsv"
    ));
    const DEFAULT_MODULE_CLOSURES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-closures.tsv"
    ));
    const DEFAULT_MODULE_LEDGER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-ledger.tsv"
    ));
    const DEFAULT_MODULE_NEGATIVES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-negatives.txt"
    ));
    const DECL_POSITION_MODULE_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-decl-position-a.txt"
    ));
    const DECL_POSITION_MODULE_LEDGER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-decl-position-a-ledger.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a.txt"
    ));
    const STATIC_NEGATIVE_MODULE_LEDGER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-ledger.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_REQUESTS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-requests.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_EXCLUSIONS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-exclusions.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_PROVENANCE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-provenance.tsv"
    ));
    const IMPORT_META_SCRIPT_ROOTS: [&str; 5] = [
        "test/language/expressions/import.meta/syntax/goal-async-function-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-async-generator-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-function-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-generator-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-script.js",
    ];
    const IMPORT_META_MODULE_NEGATIVES: [&str; 11] = [
        "test/language/expressions/import.meta/syntax/escape-sequence-import.js",
        "test/language/expressions/import.meta/syntax/escape-sequence-meta.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-rest-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-assignment-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-await-of-loop.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-in-loop.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-of-loop.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-rest-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-update-expr.js",
    ];
    const IMPORT_META_ADJACENT_EXCLUSIONS: [&str; 4] = [
        "test/language/expressions/assignmenttargettype/direct-import.meta.js",
        "test/language/expressions/assignmenttargettype/parenthesized-import.meta.js",
        "test/language/expressions/dynamic-import/assignment-expression/import-meta.js",
        "test/language/expressions/import.meta/distinct-for-each-module_FIXTURE.js",
    ];

    fn metadata(flags: &[&str], features: &[&str], includes: &[&str]) -> Metadata {
        Metadata {
            flags: flags.iter().map(|value| (*value).to_owned()).collect(),
            features: features.iter().map(|value| (*value).to_owned()).collect(),
            includes: includes.iter().map(|value| (*value).to_owned()).collect(),
            ..Metadata::default()
        }
    }

    fn generator_metadata() -> Metadata {
        metadata(&[], &["generators"], &[])
    }

    fn module_metadata(contract: ModuleMetadataContract) -> Metadata {
        Metadata {
            includes: contract
                .includes
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            flags: contract
                .flags
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            features: contract
                .features
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            negative: contract.negative.map(|negative| NegativeExpectation {
                phase: Some(negative.phase.to_owned()),
                error_type: Some(negative.error_type.to_owned()),
            }),
        }
    }

    fn audited_module_specifiers(source: &str) -> BTreeSet<String> {
        let source = source
            .find("---*/")
            .map_or(source, |end| &source[end + "---*/".len()..]);
        source
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                if !line.starts_with("import") && !line.starts_with("export") {
                    return None;
                }
                let request = if let Some(index) = line.find(" from ") {
                    line[index + " from ".len()..].trim_start()
                } else {
                    let request = line.strip_prefix("import")?;
                    let request = request.trim_start();
                    if !matches!(request.as_bytes().first(), Some(b'\'' | b'"')) {
                        return None;
                    }
                    request
                };
                let quote = request.as_bytes().first().copied()?;
                if !matches!(quote, b'\'' | b'"') {
                    return None;
                }
                let tail = &request[1..];
                let end = tail.as_bytes().iter().position(|byte| *byte == quote)?;
                Some(tail[..end].to_owned())
            })
            .collect()
    }

    fn complete_frontmatter(source: &str) -> &str {
        let Some(start) = source.find("/*---") else {
            return "";
        };
        let marker_end = start
            + source[start..]
                .find("---*/")
                .expect("Test262 frontmatter terminator")
            + "---*/".len();
        if source[marker_end..].starts_with("\r\n") {
            &source[start..marker_end + 2]
        } else if source[marker_end..].starts_with('\n') {
            &source[start..marker_end + 1]
        } else {
            &source[start..marker_end]
        }
    }

    fn normalized_audited_request(base: &str, specifier: &str) -> String {
        let relative = specifier
            .strip_prefix("./")
            .expect("the audited module cohorts use relative child requests");
        Path::new(base)
            .parent()
            .expect("module path has a parent")
            .join(relative)
            .to_string_lossy()
            .into_owned()
    }

    fn collect_non_fixture_js(dir: &Path, suite: &Path, paths: &mut BTreeSet<String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
            .map(|entry| entry.expect("read Test262 namespace entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect_non_fixture_js(&path, suite, paths);
            } else if path.extension().is_some_and(|extension| extension == "js")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with("_FIXTURE.js"))
            {
                paths.insert(
                    path.strip_prefix(suite)
                        .expect("namespace file belongs to suite")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    fn is_default_module_graph_root_name(name: &str) -> bool {
        name.ends_with(".js")
            && !name.ends_with("_FIXTURE.js")
            && ((name.starts_with("eval-export-dflt-")
                && !name.starts_with("eval-export-dflt-expr-err-"))
                || name.starts_with("eval-gtbndng-indirect-")
                || matches!(name, "eval-rqstd-once.js" | "eval-rqstd-order.js")
                || matches!(name, "eval-self-once.js" | "export-star-as-dflt.js")
                || (name.starts_with("instn-")
                    && name.contains("dflt")
                    && !name.starts_with("instn-star-props-dflt")
                    && !name.starts_with("instn-star-as-props-dflt")))
    }

    #[test]
    fn dependency_free_module_admission_is_exact_and_complete() {
        assert_eq!(DEPENDENCY_FREE_MODULE_ADMISSIONS.len(), 13);
        assert!(
            DEPENDENCY_FREE_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );

        for admission in &DEPENDENCY_FREE_MODULE_ADMISSIONS {
            let metadata = module_metadata(admission.metadata);
            assert!(metadata.is_module());
            assert!(module_metadata_matches(&metadata, admission.metadata));
            assert_eq!(
                authenticate_dependency_free_module_test(
                    Path::new(admission.path),
                    admission.source_sha256,
                    &metadata,
                    admission,
                ),
                Ok(true),
                "{}",
                admission.path
            );
        }
    }

    #[test]
    fn dependency_free_module_admission_rejects_source_and_metadata_drift() {
        let admission = &DEPENDENCY_FREE_MODULE_ADMISSIONS[0];
        let exact = module_metadata(admission.metadata);
        let source_drift = authenticate_dependency_free_module_test(
            Path::new(admission.path),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &exact,
            admission,
        )
        .unwrap_err();
        assert!(source_drift.contains("source drifted"));
        assert!(source_drift.contains(admission.source_sha256));

        let mut metadata_drift = exact;
        metadata_drift.flags.insert("async".to_owned());
        let metadata_drift = authenticate_dependency_free_module_test(
            Path::new(admission.path),
            admission.source_sha256,
            &metadata_drift,
            admission,
        )
        .unwrap_err();
        assert!(metadata_drift.contains("metadata shape drifted"));
    }

    #[test]
    fn declaration_position_module_admission_is_the_exact_natural_cohort() {
        assert_eq!(DECL_POSITION_MODULE_ADMISSIONS.len(), 86);
        assert!(
            DECL_POSITION_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            DECL_POSITION_MODULE_MANIFEST.lines().collect::<Vec<_>>(),
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .map(|admission| admission.path)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .filter(|admission| admission.path.contains("-export-"))
                .count(),
            43
        );
        assert_eq!(
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .filter(|admission| admission.path.contains("-import-"))
                .count(),
            43
        );
        assert_eq!(
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .filter(|admission| admission.metadata.features == ["generators"])
                .count(),
            12
        );

        let ledger_rows = DECL_POSITION_MODULE_LEDGER.lines().skip(1);
        assert_eq!(ledger_rows.clone().count(), 86);
        for (admission, row) in DECL_POSITION_MODULE_ADMISSIONS.iter().zip(ledger_rows) {
            let fields = row.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 9, "{} ledger width", admission.path);
            assert_eq!(fields[0], admission.path);
            assert_eq!(
                fields[1],
                if admission.path.contains("-export-") {
                    "export"
                } else {
                    "import"
                }
            );
            assert_eq!(fields[2], "");
            assert_eq!(fields[3], "module");
            assert_eq!(fields[4], admission.metadata.features.join(","));
            assert_eq!(fields[5], "parse");
            assert_eq!(fields[6], "SyntaxError");
            assert_eq!(fields[7], admission.source_sha256);
            let negative = admission.metadata.negative.expect("negative contract");
            assert_eq!(negative.phase, "parse");
            assert_eq!(negative.error_type, "SyntaxError");
        }

        let adjacent = "test/language/module-code/parse-err-export-dflt-const.js";
        assert!(
            !DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .any(|admission| admission.path == adjacent)
        );
        assert!(
            STATIC_NEGATIVE_MODULE_ADMISSIONS
                .iter()
                .any(|admission| admission.path == adjacent)
        );

        for excluded in [
            "test/language/module-code/import-attributes/import-attribute-empty.js",
            "test/language/module-code/top-level-await/await-expr-resolution.js",
        ] {
            assert_eq!(
                is_exact_dependency_free_module_test(Path::new(excluded), "", &Metadata::default()),
                Ok(false),
                "adjacent module surface was admitted: {excluded}"
            );
        }
    }

    #[test]
    fn declaration_position_module_admission_matches_the_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        for admission in &DECL_POSITION_MODULE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(admission.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", admission.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", admission.path));
            assert_eq!(
                is_exact_dependency_free_module_test(Path::new(admission.path), &source, &metadata),
                Ok(true),
                "{}",
                admission.path
            );
            assert_eq!(
                exact_module_test(&suite, Path::new(admission.path), &source, &metadata),
                Ok(Some(ExactModuleTest::DependencyFree)),
                "{}",
                admission.path
            );
        }

        let admission = &DECL_POSITION_MODULE_ADMISSIONS[0];
        let source = fs::read_to_string(suite.join(admission.path)).expect("read drift canary");
        let metadata = parse_metadata(&source).expect("parse drift canary metadata");
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(admission.path),
                &format!("{source}\n// source drift"),
                &metadata
            )
            .unwrap_err()
            .contains("source drifted")
        );
        let mut metadata_drift = metadata;
        metadata_drift.features.push("import.meta".to_owned());
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(admission.path),
                &source,
                &metadata_drift
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );
    }

    #[test]
    fn static_negative_module_admission_is_exact_sorted_and_source_authenticated() {
        assert_eq!(STATIC_NEGATIVE_MODULE_ADMISSIONS.len(), 67);
        assert!(
            STATIC_NEGATIVE_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            STATIC_NEGATIVE_MODULE_MANIFEST.lines().collect::<Vec<_>>(),
            STATIC_NEGATIVE_MODULE_ADMISSIONS
                .iter()
                .map(|admission| admission.path)
                .collect::<Vec<_>>()
        );

        let mut feature_counts = BTreeMap::new();
        for admission in &STATIC_NEGATIVE_MODULE_ADMISSIONS {
            *feature_counts
                .entry(admission.metadata.features.join(","))
                .or_insert(0usize) += 1;
            assert!(admission.metadata.includes.is_empty());
            assert_eq!(admission.metadata.flags, ["module"]);
            let negative = admission.metadata.negative.expect("negative contract");
            assert_eq!(negative.phase, "parse");
            assert_eq!(negative.error_type, "SyntaxError");
            assert_eq!(
                authenticate_dependency_free_module_test(
                    Path::new(admission.path),
                    admission.source_sha256,
                    &module_metadata(admission.metadata),
                    admission,
                ),
                Ok(true),
                "{}",
                admission.path
            );
            assert!(exact_module_graph_admission(Path::new(admission.path)).is_none());
        }
        assert_eq!(
            feature_counts,
            BTreeMap::from([
                (String::new(), 57),
                ("export-star-as-namespace-from-module".to_owned(), 4),
                ("generators".to_owned(), 3),
                ("let".to_owned(), 1),
                ("let,const".to_owned(), 1),
                ("new.target".to_owned(), 1),
            ])
        );

        let ledger_rows = STATIC_NEGATIVE_MODULE_LEDGER
            .lines()
            .skip(1)
            .map(|row| {
                let fields = row.split('\t').collect::<Vec<_>>();
                assert_eq!(fields.len(), 9, "{} ledger width", fields[0]);
                (fields[0], fields)
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(ledger_rows.len(), 67);
        for admission in &STATIC_NEGATIVE_MODULE_ADMISSIONS {
            let fields = &ledger_rows[admission.path];
            assert_eq!(fields[1], "");
            assert_eq!(fields[2], "module");
            assert_eq!(fields[3], admission.metadata.features.join(","));
            assert_eq!(fields[4], "parse");
            assert_eq!(fields[5], "SyntaxError");
            assert!(matches!(fields[6], "0" | "1"));
            assert_eq!(fields[7], admission.source_sha256);
            assert_eq!(fields[8].len(), 64);
        }

        let mut request_rows = BTreeMap::<&str, Vec<(usize, &str)>>::new();
        for row in STATIC_NEGATIVE_MODULE_REQUESTS.lines().skip(1) {
            let fields = row.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 3);
            assert!(ledger_rows.contains_key(fields[0]));
            request_rows
                .entry(fields[0])
                .or_default()
                .push((fields[1].parse().expect("request index"), fields[2]));
        }
        assert_eq!(request_rows.values().map(Vec::len).sum::<usize>(), 13);
        for (path, rows) in &request_rows {
            assert_eq!(rows.len(), 1, "{path}");
            assert_eq!(rows[0].0, 0, "{path}");
            assert_eq!(ledger_rows[path][6], "1", "{path}");
        }

        assert_eq!(
            STATIC_NEGATIVE_MODULE_PROVENANCE,
            concat!(
                "metric\tvalue\n",
                "selector\tincludes=[];flags=[module];negative=parse/SyntaxError;features in {[],[export-star-as-namespace-from-module],[generators],[let],[let,const],[new.target]};subtract prior audited negatives\n",
                "parent_profile_sha256\t364f45501f0b3655e801200b4e1ecb24040384a73489da1994528c911574e362\n",
                "parent_audited_negatives\t1450\n",
                "selected_roots\t67\n",
                "manifest_sha256\tdd8e65fab5447123ad48aa383a835893b72a5e899d34d2dce3a81660bdacc145\n",
            )
        );

        let mut surfaces = BTreeMap::new();
        let exclusions = STATIC_NEGATIVE_MODULE_EXCLUSIONS
            .lines()
            .skip(1)
            .map(|row| row.split('\t').collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(exclusions.len(), 25);
        for fields in &exclusions {
            assert_eq!(fields.len(), 10, "{} exclusion width", fields[1]);
            *surfaces.entry(fields[0]).or_insert(0usize) += 1;
            assert!(!ledger_rows.contains_key(fields[1]));
            assert_ne!(fields[2], "selected");
            assert_eq!(fields[8].len(), 64);
            assert_eq!(fields[9].len(), 64);
        }
        assert_eq!(
            surfaces,
            BTreeMap::from([
                ("adjacent-syntax", 4),
                ("class-private", 3),
                ("dynamic-import", 1),
                ("hidden-dynamic-import", 2),
                ("import-attributes", 3),
                ("import-defer", 2),
                ("source-phase-import", 2),
                ("top-level-await", 8),
            ])
        );
    }

    #[test]
    fn static_negative_module_admission_matches_the_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let ledger_rows = STATIC_NEGATIVE_MODULE_LEDGER
            .lines()
            .skip(1)
            .map(|row| {
                let fields = row.split('\t').collect::<Vec<_>>();
                (fields[0], fields)
            })
            .collect::<BTreeMap<_, _>>();
        let mut request_rows = BTreeMap::<&str, BTreeSet<String>>::new();
        for row in STATIC_NEGATIVE_MODULE_REQUESTS.lines().skip(1) {
            let fields = row.split('\t').collect::<Vec<_>>();
            request_rows
                .entry(fields[0])
                .or_default()
                .insert(fields[2].to_owned());
        }

        for admission in &STATIC_NEGATIVE_MODULE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(admission.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", admission.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", admission.path));
            assert_eq!(
                is_exact_dependency_free_module_test(Path::new(admission.path), &source, &metadata),
                Ok(true),
                "{}",
                admission.path
            );
            assert_eq!(
                exact_module_test(&suite, Path::new(admission.path), &source, &metadata),
                Ok(Some(ExactModuleTest::DependencyFree)),
                "{}",
                admission.path
            );
            assert_eq!(
                source_sha256(complete_frontmatter(&source)).unwrap(),
                ledger_rows[admission.path][8],
                "{} frontmatter",
                admission.path
            );
            assert_eq!(
                audited_module_specifiers(&source),
                request_rows
                    .get(admission.path)
                    .cloned()
                    .unwrap_or_default(),
                "{} static requests",
                admission.path
            );
        }

        let admission = STATIC_NEGATIVE_MODULE_ADMISSIONS
            .iter()
            .find(|admission| {
                admission.path == "test/language/export/escaped-as-export-specifier.js"
            })
            .expect("request-shaped drift canary");
        let source = fs::read_to_string(suite.join(admission.path)).expect("read drift canary");
        let metadata = parse_metadata(&source).expect("parse drift canary metadata");
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(admission.path),
                &format!("{source}\n// source drift"),
                &metadata
            )
            .unwrap_err()
            .contains("source drifted")
        );
        let mut metadata_drift = metadata;
        metadata_drift.flags.insert("generated".to_owned());
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(admission.path),
                &source,
                &metadata_drift
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        for fields in STATIC_NEGATIVE_MODULE_EXCLUSIONS
            .lines()
            .skip(1)
            .map(|row| row.split('\t').collect::<Vec<_>>())
        {
            let source = fs::read_to_string(suite.join(fields[1]))
                .unwrap_or_else(|error| panic!("read {}: {error}", fields[1]));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", fields[1]));
            assert_eq!(
                source_sha256(&source).unwrap(),
                fields[8],
                "{} source",
                fields[1]
            );
            assert_eq!(
                source_sha256(complete_frontmatter(&source)).unwrap(),
                fields[9],
                "{} frontmatter",
                fields[1]
            );
            assert_eq!(
                is_exact_dependency_free_module_test(Path::new(fields[1]), &source, &metadata),
                Ok(false),
                "excluded surface entered dependency-free admission: {}",
                fields[1]
            );
        }
    }

    #[test]
    fn fixture_graph_module_admission_is_exact_sorted_and_closed() {
        assert_eq!(FIXTURE_GRAPH_MODULE_ADMISSIONS.len(), 4);
        assert!(
            FIXTURE_GRAPH_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].root_path < pair[1].root_path)
        );
        assert_eq!(
            FIXTURE_GRAPH_MODULE_ADMISSIONS
                .iter()
                .map(|admission| admission.files.len())
                .sum::<usize>(),
            9
        );

        let mut all_paths = BTreeSet::new();
        for admission in &FIXTURE_GRAPH_MODULE_ADMISSIONS {
            assert_eq!(admission.files[0].path, admission.root_path);
            assert!(module_metadata(admission.files[0].metadata).is_module());
            let mut reachable = BTreeSet::new();
            let mut pending = vec![admission.root_path];
            while let Some(path) = pending.pop() {
                assert!(reachable.insert(path), "duplicate or cyclic edge at {path}");
                let file = admission
                    .files
                    .iter()
                    .find(|file| file.path == path)
                    .expect("every request target stays in its admission");
                assert!(all_paths.insert(file.path), "duplicate file {}", file.path);
                for request in file.requests.iter().rev() {
                    assert!(request.specifier.starts_with("./"));
                    pending.push(request.normalized_path);
                }
            }
            assert_eq!(reachable.len(), admission.files.len());
            assert!(
                admission.files[1..]
                    .iter()
                    .all(|file| module_metadata_matches(&Metadata::default(), file.metadata))
            );
        }
    }

    #[test]
    fn default_module_admission_is_exact_sorted_and_closed() {
        assert_eq!(DEFAULT_MODULE_ROOT_ADMISSIONS.len(), 38);
        assert_eq!(DEFAULT_MODULE_FILE_ADMISSIONS.len(), 58);
        assert!(
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            DEFAULT_MODULE_FILE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            DEFAULT_MODULE_MANIFEST.lines().collect::<Vec<_>>(),
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            DEFAULT_MODULE_SOURCES.lines().collect::<Vec<_>>(),
            DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.path)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.requests.len())
                .sum::<usize>(),
            43
        );

        let mut union = BTreeSet::new();
        let mut rooted_request_count = 0;
        let mut self_edge_count = 0;
        let mut expected_edges =
            String::from("root_path\tbase_path\trequest_index\tspecifier\tnormalized_path\n");
        let mut expected_closures = String::from("root_path\tclosure_files\trequest_edges\n");
        for root in &DEFAULT_MODULE_ROOT_ADMISSIONS {
            let admission = ExactModuleGraphAdmission {
                root_path: root.path,
                files: &DEFAULT_MODULE_FILE_ADMISSIONS,
                closure_file_count: root.closure_file_count,
            };
            let reachable = reachable_module_graph_paths(admission)
                .unwrap_or_else(|error| panic!("{}: {error}", root.path));
            assert_eq!(reachable.len(), root.closure_file_count, "{}", root.path);
            union.extend(reachable.iter().copied());

            let root_file = DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == root.path)
                .expect("every default cohort root is in the source ledger");
            assert!(module_metadata(root_file.metadata).is_module());

            let mut closure_requests = 0;
            for path in reachable {
                let file = DEFAULT_MODULE_FILE_ADMISSIONS
                    .iter()
                    .find(|file| file.path == path)
                    .expect("every reachable file is in the source ledger");
                for (request_index, request) in file.requests.iter().enumerate() {
                    closure_requests += 1;
                    rooted_request_count += 1;
                    if file.path == request.normalized_path {
                        self_edge_count += 1;
                    }
                    expected_edges.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        root.path,
                        file.path,
                        request_index,
                        request.specifier,
                        request.normalized_path
                    ));
                }
            }
            expected_closures.push_str(&format!(
                "{}\t{}\t{}\n",
                root.path, root.closure_file_count, closure_requests
            ));
        }
        assert_eq!(union.len(), 58);
        assert_eq!(
            union,
            DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.path)
                .collect()
        );
        assert_eq!(rooted_request_count, 45);
        assert_eq!(self_edge_count, 21);
        assert_eq!(expected_edges, DEFAULT_MODULE_EDGES);
        assert_eq!(expected_closures, DEFAULT_MODULE_CLOSURES);

        for file in &DEFAULT_MODULE_FILE_ADMISSIONS {
            assert_eq!(file.source_sha256.len(), 64, "{}", file.path);
            assert!(
                file.source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}",
                file.path
            );
            if file.path.ends_with("_FIXTURE.js") {
                assert!(module_metadata_matches(&Metadata::default(), file.metadata));
            }
            let mut specifiers = BTreeSet::new();
            for request in file.requests {
                assert!(
                    specifiers.insert(request.specifier),
                    "duplicate request {} in {}",
                    request.specifier,
                    file.path
                );
                assert_eq!(
                    request.normalized_path,
                    normalized_audited_request(file.path, request.specifier),
                    "{} -> {}",
                    file.path,
                    request.specifier
                );
                assert!(
                    DEFAULT_MODULE_FILE_ADMISSIONS
                        .iter()
                        .any(|candidate| candidate.path == request.normalized_path),
                    "{} -> {}",
                    file.path,
                    request.normalized_path
                );
            }
        }

        let audited_negatives = DEFAULT_MODULE_ROOT_ADMISSIONS
            .iter()
            .filter_map(|root| {
                DEFAULT_MODULE_FILE_ADMISSIONS
                    .iter()
                    .find(|file| file.path == root.path)
                    .filter(|file| file.metadata.negative.is_some())
                    .map(|file| file.path)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            audited_negatives,
            DEFAULT_MODULE_NEGATIVES.lines().collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_module_admission_matches_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let module_dir = suite.join("test/language/module-code");
        let natural_roots = fs::read_dir(&module_dir)
            .expect("read pinned module-code directory")
            .map(|entry| entry.expect("read pinned module-code entry"))
            .filter(|entry| entry.path().is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_default_module_graph_root_name(name))
            .map(|name| format!("test/language/module-code/{name}"))
            .collect::<BTreeSet<_>>();
        assert_eq!(natural_roots.len(), 38);
        assert_eq!(
            natural_roots,
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.to_owned())
                .collect()
        );

        let ledger = DEFAULT_MODULE_LEDGER
            .lines()
            .skip(1)
            .map(|line| {
                let fields = line.split('\t').collect::<Vec<_>>();
                assert_eq!(fields.len(), 9, "{line}");
                (fields[0], fields)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(ledger.len(), 58);
        for file in &DEFAULT_MODULE_FILE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(file.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", file.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", file.path));
            authenticate_module_graph_file(Path::new(file.path), &source, &metadata, file)
                .unwrap_or_else(|error| panic!("authenticate {}: {error}", file.path));
            assert_eq!(
                audited_module_specifiers(&source),
                file.requests
                    .iter()
                    .map(|request| request.specifier.to_owned())
                    .collect(),
                "{} static requests drifted",
                file.path
            );

            let fields = ledger.get(file.path).expect("source ledger row");
            assert_eq!(
                fields[1],
                if file.path.ends_with("_FIXTURE.js") {
                    "fixture"
                } else {
                    "root"
                }
            );
            assert_eq!(fields[2], file.metadata.includes.join(","));
            assert_eq!(fields[3], file.metadata.flags.join(","));
            assert_eq!(fields[4], file.metadata.features.join(","));
            assert_eq!(
                fields[5],
                file.metadata.negative.map_or("", |negative| negative.phase)
            );
            assert_eq!(
                fields[6],
                file.metadata
                    .negative
                    .map_or("", |negative| negative.error_type)
            );
            assert_eq!(fields[7], file.source_sha256);
            assert_eq!(
                fields[8],
                source_sha256(complete_frontmatter(&source)).expect("hash frontmatter")
            );
        }

        for root in &DEFAULT_MODULE_ROOT_ADMISSIONS {
            let source = fs::read_to_string(suite.join(root.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", root.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", root.path));
            assert_eq!(
                exact_module_test(&suite, Path::new(root.path), &source, &metadata),
                Ok(Some(ExactModuleTest::FixtureGraph)),
                "{}",
                root.path
            );
        }
    }

    #[test]
    fn default_module_admission_rejects_drift_and_preserves_adjacent_exclusions() {
        let file = DEFAULT_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path == "test/language/module-code/export-star-as-dflt.js")
            .expect("audited default-export root");
        let exact = module_metadata(file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &exact,
                file,
            )
            .unwrap_err()
            .contains("source drifted")
        );
        let mut metadata_drift = exact;
        metadata_drift.features.clear();
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                file.source_sha256,
                &metadata_drift,
                file,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        for excluded in [
            "test/language/expressions/dynamic-import/always-create-new-promise.js",
            "test/language/module-code/top-level-await/await-expr-resolution.js",
            "test/language/module-code/import-attributes/import-attribute-empty.js",
            "test/language/module-code/source-phase-import/import-source.js",
        ] {
            assert!(
                exact_module_graph_admission(Path::new(excluded)).is_none(),
                "excluded module surface was admitted: {excluded}"
            );
        }
    }

    #[test]
    fn import_meta_module_admission_is_the_exact_closed_module_goal_cohort() {
        assert_eq!(IMPORT_META_MODULE_ROOT_ADMISSIONS.len(), 17);
        assert_eq!(IMPORT_META_MODULE_FILE_ADMISSIONS.len(), 18);
        assert!(
            IMPORT_META_MODULE_ROOT_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            IMPORT_META_MODULE_FILE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            IMPORT_META_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.requests.len())
                .sum::<usize>(),
            1
        );

        let root_paths = IMPORT_META_MODULE_ROOT_ADMISSIONS
            .iter()
            .map(|root| root.path)
            .collect::<BTreeSet<_>>();
        let file_paths = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .map(|file| file.path)
            .collect::<BTreeSet<_>>();
        assert!(root_paths.is_subset(&file_paths));
        assert_eq!(
            file_paths
                .difference(&root_paths)
                .copied()
                .collect::<Vec<_>>(),
            ["test/language/expressions/import.meta/distinct-for-each-module_FIXTURE.js",]
        );

        let mut union = BTreeSet::new();
        let mut rooted_request_count = 0;
        for root in &IMPORT_META_MODULE_ROOT_ADMISSIONS {
            assert!(!root.path.ends_with("_FIXTURE.js"));
            let admission = exact_module_graph_admission(Path::new(root.path))
                .expect("every import.meta module root has an exact graph admission");
            assert_eq!(admission.root_path, root.path);
            assert_eq!(admission.closure_file_count, root.closure_file_count);
            assert_eq!(admission.files.len(), 18);

            let reachable = reachable_module_graph_paths(admission)
                .unwrap_or_else(|error| panic!("{}: {error}", root.path));
            assert_eq!(reachable.len(), root.closure_file_count, "{}", root.path);
            assert_eq!(
                root.closure_file_count,
                if root.path.ends_with("/distinct-for-each-module.js") {
                    2
                } else {
                    1
                },
                "{}",
                root.path
            );
            union.extend(reachable.iter().copied());
            rooted_request_count += reachable
                .iter()
                .map(|path| {
                    IMPORT_META_MODULE_FILE_ADMISSIONS
                        .iter()
                        .find(|file| file.path == *path)
                        .expect("reachable import.meta source is authenticated")
                        .requests
                        .len()
                })
                .sum::<usize>();

            let root_file = IMPORT_META_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == root.path)
                .expect("every import.meta root is in the source table");
            let metadata = module_metadata(root_file.metadata);
            assert!(metadata.is_module());
            assert_eq!(
                metadata.features.first().map(String::as_str),
                Some("import.meta")
            );
        }
        assert_eq!(union, file_paths);
        assert_eq!(rooted_request_count, 1);

        for file in &IMPORT_META_MODULE_FILE_ADMISSIONS {
            assert_eq!(file.source_sha256.len(), 64, "{}", file.path);
            assert!(
                file.source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}",
                file.path
            );
            if file.path.ends_with("_FIXTURE.js") {
                assert!(module_metadata_matches(&Metadata::default(), file.metadata));
            }
            let mut specifiers = BTreeSet::new();
            for request in file.requests {
                assert!(specifiers.insert(request.specifier));
                assert_eq!(
                    request.normalized_path,
                    normalized_audited_request(file.path, request.specifier)
                );
                assert!(file_paths.contains(request.normalized_path));
            }
        }

        let negatives = IMPORT_META_MODULE_ROOT_ADMISSIONS
            .iter()
            .filter_map(|root| {
                IMPORT_META_MODULE_FILE_ADMISSIONS
                    .iter()
                    .find(|file| file.path == root.path)
                    .filter(|file| file.metadata.negative.is_some())
                    .map(|file| file.path)
            })
            .collect::<Vec<_>>();
        assert_eq!(negatives, IMPORT_META_MODULE_NEGATIVES);
        for negative in negatives {
            let contract = IMPORT_META_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == negative)
                .expect("audited import.meta negative")
                .metadata;
            let expected = contract.negative.expect("negative metadata contract");
            assert_eq!(expected.phase, "parse", "{negative}");
            assert_eq!(expected.error_type, "SyntaxError", "{negative}");
        }

        for script in IMPORT_META_SCRIPT_ROOTS {
            assert!(
                exact_module_graph_admission(Path::new(script)).is_none(),
                "{script}"
            );
        }
        for excluded in IMPORT_META_ADJACENT_EXCLUSIONS {
            assert!(
                exact_module_graph_admission(Path::new(excluded)).is_none(),
                "adjacent import.meta surface was admitted: {excluded}"
            );
        }
    }

    #[test]
    fn import_meta_module_admission_matches_the_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let mut natural_roots = BTreeSet::new();
        collect_non_fixture_js(
            &suite.join("test/language/expressions/import.meta"),
            &suite,
            &mut natural_roots,
        );
        assert_eq!(natural_roots.len(), 22);

        let mut module_roots = BTreeSet::new();
        let mut script_roots = BTreeSet::new();
        for path in &natural_roots {
            let source = fs::read_to_string(suite.join(path))
                .unwrap_or_else(|error| panic!("read {path}: {error}"));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {path} metadata: {error}"));
            assert!(
                metadata
                    .features
                    .iter()
                    .any(|feature| feature == "import.meta")
            );
            if metadata.is_module() {
                module_roots.insert(path.to_owned());
            } else {
                script_roots.insert(path.to_owned());
            }
        }
        assert_eq!(module_roots.len(), 17);
        assert_eq!(script_roots.len(), 5);
        assert_eq!(
            module_roots,
            IMPORT_META_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.to_owned())
                .collect()
        );
        assert_eq!(
            script_roots,
            IMPORT_META_SCRIPT_ROOTS
                .into_iter()
                .map(str::to_owned)
                .collect()
        );

        for file in &IMPORT_META_MODULE_FILE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(file.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", file.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", file.path));
            authenticate_module_graph_file(Path::new(file.path), &source, &metadata, file)
                .unwrap_or_else(|error| panic!("authenticate {}: {error}", file.path));
            assert_eq!(
                audited_module_specifiers(&source),
                file.requests
                    .iter()
                    .map(|request| request.specifier.to_owned())
                    .collect(),
                "{} static requests drifted",
                file.path
            );
        }

        for root in &IMPORT_META_MODULE_ROOT_ADMISSIONS {
            let source = fs::read_to_string(suite.join(root.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", root.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", root.path));
            assert_eq!(
                exact_module_test(&suite, Path::new(root.path), &source, &metadata),
                Ok(Some(ExactModuleTest::FixtureGraph)),
                "{}",
                root.path
            );
        }

        for script in IMPORT_META_SCRIPT_ROOTS {
            let source = fs::read_to_string(suite.join(script))
                .unwrap_or_else(|error| panic!("read {script}: {error}"));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {script} metadata: {error}"));
            assert_eq!(
                exact_module_test(&suite, Path::new(script), &source, &metadata),
                Ok(None),
                "script-goal import.meta root was admitted as a module: {script}"
            );
        }

        let dynamic_import = IMPORT_META_ADJACENT_EXCLUSIONS[2];
        let source = fs::read_to_string(suite.join(dynamic_import))
            .unwrap_or_else(|error| panic!("read {dynamic_import}: {error}"));
        let metadata = parse_metadata(&source)
            .unwrap_or_else(|error| panic!("parse {dynamic_import} metadata: {error}"));
        assert!(metadata.is_module());
        assert!(metadata.is_async());
        assert_eq!(metadata.features, ["dynamic-import", "import.meta"]);
        assert_eq!(
            exact_module_test(&suite, Path::new(dynamic_import), &source, &metadata),
            Ok(None)
        );

        for assignment_target in &IMPORT_META_ADJACENT_EXCLUSIONS[..2] {
            let source = fs::read_to_string(suite.join(assignment_target))
                .unwrap_or_else(|error| panic!("read {assignment_target}: {error}"));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {assignment_target} metadata: {error}"));
            assert!(!metadata.is_module());
            assert!(metadata.features.is_empty());
            assert_eq!(
                metadata
                    .negative
                    .as_ref()
                    .and_then(|negative| negative.phase.as_deref()),
                Some("parse")
            );
            assert_eq!(
                metadata
                    .negative
                    .as_ref()
                    .and_then(|negative| negative.error_type.as_deref()),
                Some("SyntaxError")
            );
            assert_eq!(
                exact_module_test(&suite, Path::new(assignment_target), &source, &metadata,),
                Ok(None)
            );
        }
    }

    #[test]
    fn import_meta_module_admission_rejects_every_authenticated_dimension_drift() {
        let root = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path.ends_with("/import-meta-is-an-ordinary-object.js"))
            .expect("positive import.meta root");
        let exact = module_metadata(root.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(root.path),
                root.source_sha256,
                &exact,
                root,
            ),
            Ok(())
        );
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(root.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &exact,
                root,
            )
            .unwrap_err()
            .contains("source drifted")
        );

        let mut feature_drift = exact;
        feature_drift.features.clear();
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(root.path),
                root.source_sha256,
                &feature_drift,
                root,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );
        assert!(
            authenticate_module_graph_file_digest(
                Path::new("test/language/expressions/import.meta/unlisted.js"),
                root.source_sha256,
                &module_metadata(root.metadata),
                root,
            )
            .unwrap_err()
            .contains("path drifted")
        );

        let negative = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path.ends_with("/escape-sequence-import.js"))
            .expect("negative import.meta root");
        let mut negative_metadata = module_metadata(negative.metadata);
        negative_metadata
            .negative
            .as_mut()
            .expect("negative metadata")
            .phase = Some("resolution".to_owned());
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(negative.path),
                negative.source_sha256,
                &negative_metadata,
                negative,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        let fixture = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path.ends_with("_FIXTURE.js"))
            .expect("import.meta fixture");
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(fixture.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &Metadata::default(),
                fixture,
            )
            .unwrap_err()
            .contains("source drifted")
        );

        let distinct = IMPORT_META_MODULE_ROOT_ADMISSIONS[0];
        let closure_drift = authenticate_exact_module_graph_closure(
            ExactModuleGraphAdmission {
                root_path: distinct.path,
                files: &IMPORT_META_MODULE_FILE_ADMISSIONS,
                closure_file_count: 1,
            },
            |_| panic!("closure size drift must fail before reading source files"),
        )
        .unwrap_err();
        assert!(closure_drift.contains("closure size drifted"));

        let request = IMPORT_META_MODULE_FILE_ADMISSIONS[0].requests[0];
        assert_eq!(
            normalize_exact_module_request(
                Path::new(distinct.path),
                distinct.path,
                request.specifier,
            ),
            Ok(request.normalized_path.to_owned())
        );
        assert!(
            normalize_exact_module_request(
                Path::new(distinct.path),
                distinct.path,
                "./unlisted_FIXTURE.js",
            )
            .unwrap_err()
            .contains("unaudited request")
        );
        assert!(
            normalize_exact_module_request(
                Path::new(distinct.path),
                "test/language/expressions/import.meta/unlisted.js",
                request.specifier,
            )
            .unwrap_err()
            .contains("unaudited base")
        );

        for excluded in IMPORT_META_ADJACENT_EXCLUSIONS {
            assert!(exact_module_graph_admission(Path::new(excluded)).is_none());
        }
    }

    #[test]
    fn namespace_module_admission_is_the_exact_natural_closed_cohort() {
        assert_eq!(NAMESPACE_MODULE_ROOT_ADMISSIONS.len(), 37);
        assert_eq!(NAMESPACE_MODULE_FILE_ADMISSIONS.len(), 48);
        assert!(
            NAMESPACE_MODULE_ROOT_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            NAMESPACE_MODULE_FILE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            NAMESPACE_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.requests.len())
                .sum::<usize>(),
            46
        );

        let mut union = BTreeSet::new();
        for root in &NAMESPACE_MODULE_ROOT_ADMISSIONS {
            assert!(!root.path.ends_with("_FIXTURE.js"));
            let admission = ExactModuleGraphAdmission {
                root_path: root.path,
                files: &NAMESPACE_MODULE_FILE_ADMISSIONS,
                closure_file_count: root.closure_file_count,
            };
            let reachable = reachable_module_graph_paths(admission)
                .unwrap_or_else(|error| panic!("{}: {error}", root.path));
            assert_eq!(reachable.len(), root.closure_file_count, "{}", root.path);
            union.extend(reachable);

            let root_file = NAMESPACE_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == root.path)
                .expect("every namespace root is present in the file ledger");
            assert!(module_metadata(root_file.metadata).is_module());
        }
        assert_eq!(union.len(), 48);
        assert_eq!(
            union,
            NAMESPACE_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.path)
                .collect()
        );

        for file in &NAMESPACE_MODULE_FILE_ADMISSIONS {
            assert_eq!(file.source_sha256.len(), 64, "{}", file.path);
            assert!(
                file.source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}",
                file.path
            );
            let mut specifiers = BTreeSet::new();
            for request in file.requests {
                assert!(
                    specifiers.insert(request.specifier),
                    "duplicate request {} in {}",
                    request.specifier,
                    file.path
                );
                assert_eq!(
                    request.normalized_path,
                    normalized_audited_request(file.path, request.specifier),
                    "{} -> {}",
                    file.path,
                    request.specifier
                );
                assert!(
                    NAMESPACE_MODULE_FILE_ADMISSIONS
                        .iter()
                        .any(|candidate| candidate.path == request.normalized_path),
                    "{} -> {}",
                    file.path,
                    request.normalized_path
                );
            }
        }
    }

    #[test]
    fn namespace_module_admission_matches_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let mut natural_roots = BTreeSet::new();
        collect_non_fixture_js(
            &suite.join("test/language/module-code/namespace"),
            &suite,
            &mut natural_roots,
        );
        natural_roots.insert(
            "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace.js"
                .to_owned(),
        );
        assert_eq!(natural_roots.len(), 37);
        assert_eq!(
            natural_roots,
            NAMESPACE_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.to_owned())
                .collect()
        );

        for file in &NAMESPACE_MODULE_FILE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(file.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", file.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", file.path));
            authenticate_module_graph_file(Path::new(file.path), &source, &metadata, file)
                .unwrap_or_else(|error| panic!("authenticate {}: {error}", file.path));
            assert_eq!(
                audited_module_specifiers(&source),
                file.requests
                    .iter()
                    .map(|request| request.specifier.to_owned())
                    .collect(),
                "{} static requests drifted",
                file.path
            );
        }

        for root in &NAMESPACE_MODULE_ROOT_ADMISSIONS {
            let source = fs::read_to_string(suite.join(root.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", root.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", root.path));
            assert_eq!(
                exact_module_test(&suite, Path::new(root.path), &source, &metadata),
                Ok(Some(ExactModuleTest::FixtureGraph)),
                "{}",
                root.path
            );
        }
    }

    #[test]
    fn namespace_module_admission_rejects_source_metadata_and_path_drift() {
        let file = NAMESPACE_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path == "test/language/module-code/namespace/Symbol.iterator.js")
            .expect("audited namespace root");
        let exact = module_metadata(file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );

        assert!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &exact,
                file,
            )
            .unwrap_err()
            .contains("source drifted")
        );

        let mut metadata_drift = exact;
        metadata_drift.features.push("Symbol".to_owned());
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                file.source_sha256,
                &metadata_drift,
                file,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        assert!(
            authenticate_module_graph_file_digest(
                Path::new("test/language/module-code/namespace/unlisted.js"),
                file.source_sha256,
                &module_metadata(file.metadata),
                file,
            )
            .unwrap_err()
            .contains("path drifted")
        );
    }

    #[test]
    fn module_graph_admission_rejects_request_and_closure_drift() {
        const MISSING_REQUEST_FILES: [ModuleGraphFileAdmission; 1] = [ModuleGraphFileAdmission {
            path: "test/root.js",
            source_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            metadata: MODULE_METADATA,
            requests: &[],
        }];
        let missing_request = super::authenticate_exact_module_graph_closure(
            ExactModuleGraphAdmission {
                root_path: "test/root.js",
                files: &MISSING_REQUEST_FILES,
                closure_file_count: 2,
            },
            |_| panic!("closure drift must fail before reading sources"),
        )
        .unwrap_err();
        assert!(missing_request.contains("closure size drifted"));

        const ESCAPED_REQUESTS: [ModuleRequestAdmission; 1] = [ModuleRequestAdmission {
            specifier: "./escaped.js",
            normalized_path: "test/escaped.js",
        }];
        const ESCAPED_REQUEST_FILES: [ModuleGraphFileAdmission; 1] = [ModuleGraphFileAdmission {
            path: "test/root.js",
            source_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            metadata: MODULE_METADATA,
            requests: &ESCAPED_REQUESTS,
        }];
        let escaped_request = reachable_module_graph_paths(ExactModuleGraphAdmission {
            root_path: "test/root.js",
            files: &ESCAPED_REQUEST_FILES,
            closure_file_count: 1,
        })
        .unwrap_err();
        assert!(escaped_request.contains("request escaped"));
        assert!(escaped_request.contains("./escaped.js"));
    }

    #[test]
    fn fixture_graph_file_authentication_rejects_source_metadata_and_path_drift() {
        let file = &FIXTURE_GRAPH_MODULE_ADMISSIONS[0].files[1];
        let exact = module_metadata(file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );

        let source_drift = authenticate_module_graph_file_digest(
            Path::new(file.path),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &exact,
            file,
        )
        .unwrap_err();
        assert!(source_drift.contains("source drifted"));
        assert!(source_drift.contains(file.source_sha256));

        let mut drifted_metadata = exact;
        drifted_metadata.flags.insert("module".to_owned());
        let metadata_drift = authenticate_module_graph_file_digest(
            Path::new(file.path),
            file.source_sha256,
            &drifted_metadata,
            file,
        )
        .unwrap_err();
        assert!(metadata_drift.contains("metadata shape drifted"));

        let path_drift = authenticate_module_graph_file_digest(
            Path::new("test/language/module-code/other_FIXTURE.js"),
            file.source_sha256,
            &module_metadata(file.metadata),
            file,
        )
        .unwrap_err();
        assert!(path_drift.contains("path drifted"));
    }

    #[test]
    fn recursive_fixture_closure_authentication_rejects_nested_drift() {
        const ROOT_SOURCE: &str =
            "/*---\nflags: [module]\n---*/\nimport \"./fixture_FIXTURE.js\";\n";
        const FIXTURE_SOURCE: &str = "export const value = 1;\n";
        const REQUESTS: [ModuleRequestAdmission; 1] = [ModuleRequestAdmission {
            specifier: "./fixture_FIXTURE.js",
            normalized_path: "test/fixture_FIXTURE.js",
        }];
        const FILES: [ModuleGraphFileAdmission; 2] = [
            ModuleGraphFileAdmission {
                path: "test/root.js",
                source_sha256: "32d8e8b1d38a53f8f4873d89cd0d00a115c33b0ed8294eb016e22e3edea95afe",
                metadata: MODULE_METADATA,
                requests: &REQUESTS,
            },
            ModuleGraphFileAdmission {
                path: "test/fixture_FIXTURE.js",
                source_sha256: "5d8f65d2774e206bc9f7a7a4ad39ca2dc563b5c31e46ab57ef4874961237ce29",
                metadata: MODULE_FIXTURE_METADATA,
                requests: &[],
            },
        ];
        const ADMISSION: FixtureGraphModuleAdmission = FixtureGraphModuleAdmission {
            root_path: "test/root.js",
            files: &FILES,
        };

        let exact = authenticate_fixture_graph_closure(&ADMISSION, |path| match path {
            "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
            "test/fixture_FIXTURE.js" => Ok(FIXTURE_SOURCE.to_owned()),
            _ => Err(format!("unexpected path: {path}")),
        });
        assert_eq!(exact, Ok(()));

        let drift = authenticate_fixture_graph_closure(&ADMISSION, |path| match path {
            "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
            "test/fixture_FIXTURE.js" => Ok("export const value = 2;\n".to_owned()),
            _ => Err(format!("unexpected path: {path}")),
        })
        .unwrap_err();
        assert!(drift.contains("source drifted"));
        assert!(drift.contains("fixture_FIXTURE.js"));
    }

    #[test]
    fn fixture_graph_loader_normalization_rejects_unlisted_edges() {
        let admission = &FIXTURE_GRAPH_MODULE_ADMISSIONS[0];
        let base = admission.root_path;
        let request = admission.files[0].requests[0];
        assert_eq!(
            normalize_exact_module_request(Path::new(admission.root_path), base, request.specifier,),
            Ok(request.normalized_path.to_owned())
        );
        assert!(
            normalize_exact_module_request(
                Path::new(admission.root_path),
                base,
                "./unlisted_FIXTURE.js",
            )
            .unwrap_err()
            .contains("unaudited request")
        );
        assert!(
            normalize_exact_module_request(
                Path::new(admission.root_path),
                "test/language/module-code/unlisted.js",
                request.specifier,
            )
            .unwrap_err()
            .contains("unaudited base")
        );
    }

    #[test]
    fn ordinary_module_is_not_admitted() {
        let metadata = metadata(&["module"], &[], &[]);
        assert_eq!(
            is_exact_dependency_free_module_test(
                Path::new("test/language/module-code/not-a-pinned-root.js"),
                "export {};",
                &metadata,
            ),
            Ok(false)
        );
        assert_eq!(
            exact_module_test(
                Path::new("."),
                Path::new("test/language/module-code/not-a-pinned-root.js"),
                "export {};",
                &metadata,
            ),
            Ok(None)
        );
        assert_ne!(
            exact_module_test(
                Path::new("."),
                Path::new(DEPENDENCY_FREE_MODULE_ADMISSIONS[0].path),
                "drifted",
                &module_metadata(DEPENDENCY_FREE_MODULE_ADMISSIONS[0].metadata),
            ),
            Ok(Some(ExactModuleTest::DependencyFree))
        );
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/language/module-code/not-a-pinned-root.js"),
                "export {};",
                &metadata,
                false,
            ),
            ["module"]
        );
    }

    #[test]
    fn agent_host_admission_ledger_is_exact_sorted_and_metadata_frozen() {
        assert_eq!(AGENT_HOST_ADMISSIONS.len(), 59);
        assert!(
            AGENT_HOST_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );

        let broadcast = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent broadcast cohort A")
            .collect::<Vec<_>>();
        assert_eq!(broadcast.len(), 15);
        let ledger = broadcast
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&ledger).unwrap(),
            "b467b2cdca29ad877981b7894e5b28bdf966385034aa5e722d9d81b86b19c0cf"
        );

        let mut feature_shapes = BTreeSet::new();
        for admission in broadcast {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut drifted = exact.clone();
            drifted.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&drifted, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 3);

        let bounded_wait = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent bounded wait cohort A")
            .collect::<Vec<_>>();
        assert_eq!(bounded_wait.len(), 22);
        let ledger = bounded_wait
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&ledger).unwrap(),
            "79105013edd054a045fe16f3de55fe1b5fb233e373ac9052c1213f1c4bcea04d"
        );
        let mut feature_shapes = BTreeSet::new();
        for admission in bounded_wait {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut include_drift = exact.clone();
            include_drift.includes.push("extra.js".to_owned());
            assert!(!agent_host_metadata_matches(&include_drift, admission));

            let mut flag_drift = exact.clone();
            flag_drift.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&flag_drift, admission));

            let mut negative_drift = exact.clone();
            negative_drift.negative = Some(Default::default());
            assert!(!agent_host_metadata_matches(&negative_drift, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 2);

        let wake_count_location = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent wake/count/location cohort")
            .collect::<Vec<_>>();
        assert_eq!(wake_count_location.len(), 17);
        let source_ledger = wake_count_location
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&source_ledger).unwrap(),
            "04625efdf79624f49c5bcc24282eae8962ba29294b4e3be6b39958083763e472"
        );
        let metadata_ledger = wake_count_location
            .iter()
            .map(|admission| {
                format!(
                    "{}\tflags=-\tfeatures={}\tincludes=atomicsHelper.js\tnegative=-\n",
                    admission.path,
                    admission.features.join(",")
                )
            })
            .collect::<String>();
        assert_eq!(
            source_sha256(&metadata_ledger).unwrap(),
            "bcf9a3992212ea0dcfb401b5205dbe3afbaa21c2c8f9d459e413c0845a36897c"
        );
        let mut feature_shapes = BTreeSet::new();
        for admission in wake_count_location {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut include_drift = exact.clone();
            include_drift.includes.push("extra.js".to_owned());
            assert!(!agent_host_metadata_matches(&include_drift, admission));

            let mut flag_drift = exact.clone();
            flag_drift.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&flag_drift, admission));

            let mut negative_drift = exact.clone();
            negative_drift.negative = Some(Default::default());
            assert!(!agent_host_metadata_matches(&negative_drift, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 2);

        let fifo_wake_order = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent FIFO wake-order cohort")
            .collect::<Vec<_>>();
        assert_eq!(fifo_wake_order.len(), 4);
        let source_ledger = fifo_wake_order
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&source_ledger).unwrap(),
            "6881f53503b504225342ba6611216642a6799f099255f7b6846b762b2865d358"
        );
        let metadata_ledger = fifo_wake_order
            .iter()
            .map(|admission| {
                format!(
                    "{}\tflags=-\tfeatures={}\tincludes=atomicsHelper.js\tnegative=-\n",
                    admission.path,
                    admission.features.join(",")
                )
            })
            .collect::<String>();
        assert_eq!(
            source_sha256(&metadata_ledger).unwrap(),
            "6f22656e524ec7736801c3e6a46d469c153da77437735d5fd348e0480c9ac8f7"
        );
        let mut feature_shapes = BTreeSet::new();
        for admission in fifo_wake_order {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut include_drift = exact.clone();
            include_drift.includes.push("extra.js".to_owned());
            assert!(!agent_host_metadata_matches(&include_drift, admission));

            let mut flag_drift = exact.clone();
            flag_drift.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&flag_drift, admission));

            let mut negative_drift = exact.clone();
            negative_drift.negative = Some(Default::default());
            assert!(!agent_host_metadata_matches(&negative_drift, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 2);

        let stage_a = AGENT_HOST_ADMISSIONS
            .iter()
            .find(|admission| admission.cohort == "Test262 agent Stage A")
            .unwrap();
        assert_eq!(stage_a.path, "test/built-ins/Atomics/wait/good-views.js");
        assert_eq!(
            stage_a.source_sha256,
            "7ab45f324e0f668a9d9f3df03c866b0ac32276eb1dfb649d1e5783a88f70bb21"
        );
        assert!(agent_host_metadata_matches(
            &metadata(&[], &["Atomics"], &["atomicsHelper.js"]),
            stage_a
        ));
    }

    #[test]
    fn combines_modes_flags_features_includes_and_hooks_in_stable_order() {
        let metadata = metadata(
            &["module", "async", "CanBlockIsFalse"],
            &["host-gc-required", "IsHTMLDDA"],
            &["atomicsHelper.js", "detachArrayBuffer.js"],
        );
        let actual = missing_host_capability_hints(
            Path::new("test/example.js"),
            "$262.createRealm(); $262.evalScript('0'); $262.gc();",
            &metadata,
            false,
        );
        assert_eq!(
            actual,
            [
                "agent",
                "async",
                "can-block:false",
                "create-realm",
                "detach-array-buffer",
                "eval-script",
                "gc",
                "is-html-dda",
                "module",
            ]
        );
    }

    #[test]
    fn can_block_true_is_the_supported_default_and_is_not_missing() {
        let metadata = metadata(&["CanBlockIsTrue"], &[], &[]);
        assert!(
            missing_host_capability_hints(Path::new("test/example.js"), "0;", &metadata, false)
                .is_empty()
        );
    }

    #[test]
    fn scoped_async_host_removes_only_the_async_execution_gap() {
        let metadata = metadata(&["module", "async"], &[], &[]);
        assert_eq!(
            missing_host_capability_hints(Path::new("test/example.js"), "0;", &metadata, true,),
            ["module"]
        );
    }

    #[test]
    fn declared_module_remains_the_authoritative_execution_gap() {
        let metadata = metadata(&["module"], &["generators"], &[]);
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                "const callable = async () => 1;",
                &metadata,
                false,
            ),
            ["module"]
        );
        assert!(generator_destructuring_source_needs_async_guard(
            "const callable = async () => 1;",
            &metadata,
        ));
    }

    #[test]
    fn generator_admission_guard_detects_async_functions_and_arrows() {
        let metadata = generator_metadata();
        let sources = [
            "async function ordinary() {}",
            "const generator = async function* () {};",
            "const arrow = async value => value;",
            "const arrow = async (value, nested = (item => item)) => value;",
            "async function outer() { function* nested() { yield 1; } }",
            "const from_substitution = `${async function () {}}`;",
        ];

        for source in sources {
            assert!(
                generator_destructuring_source_needs_async_guard(source, &metadata),
                "source should require the scoped async guard: {source}",
            );
        }
    }

    #[test]
    fn generator_admission_guard_is_feature_scoped_and_skips_hidden_text() {
        let metadata = generator_metadata();
        let sources = [
            "var async = 1;",
            "async(value);",
            "({ async() {} });",
            "async['computed']();",
            "// async function commented() {}\n0;",
            "/* async value => value */ 0;",
            "'async function inString() {}';",
            "\"async value => value\";",
            "`async function inTemplateRaw() {}; async value => value`;",
            "const expression = /async function inPattern() {}/;",
            "const expressions = [/async value => value/, /async\\s+function/gi];",
        ];

        for source in sources {
            assert!(
                !generator_destructuring_source_needs_async_guard(source, &metadata),
                "source should not require the scoped async guard: {source}",
            );
        }
        assert!(!generator_destructuring_source_needs_async_guard(
            "async function outside_the_cohort() {}",
            &Metadata::default(),
        ));
    }

    #[test]
    fn scoped_async_heads_honor_no_line_terminator_restrictions() {
        let metadata = generator_metadata();
        let sources = [
            "async\nfunction split() {}",
            "async\r\nfunction split() {}",
            "async\u{2028}function split() {}",
            "async\u{2029}value => value",
            "async\nvalue => value",
            "async value\n=> value",
            "async\n(value) => value",
            "async (value)\n=> value",
            "({ async\nmethod() {} });",
            "({ async\n*generatorMethod() {} });",
            "async /* comment with\nline */ function split() {}",
        ];

        for source in sources {
            assert!(
                !generator_destructuring_source_needs_async_guard(source, &metadata),
                "line terminator should split the async callable head: {source:?}",
            );
        }

        for source in [
            "async /* comment */ function joined() {}",
            "async /* comment */ value /* comment */ => value",
            "async /* comment */ (value) /* comment */ => value",
        ] {
            assert!(
                generator_destructuring_source_needs_async_guard(source, &metadata),
                "comment trivia without a line terminator should preserve the head: {source}",
            );
        }
    }

    #[test]
    fn scanner_skips_comments_quoted_strings_and_template_raw_text() {
        let source = r#"
            // $262.gc()
            /* $262.agent.start('') */
            '$262.createRealm()';
            "$262.evalScript('0')";
            `$262.detachArrayBuffer(buffer) ${$262.IsHTMLDDA}`;
            `outer ${`inner raw $262.gc ${$262.AbstractModuleSource}`}`;
        "#;
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["abstract-module-source", "is-html-dda"]
        );
    }

    #[test]
    fn scanner_accepts_trivia_around_member_access_and_deduplicates() {
        let source = "$262 /* a */ . // b\n gc(); $262.gc();";
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["gc"]
        );
    }

    #[test]
    fn host_scanner_does_not_hide_a_hook_behind_the_regexp_heuristic() {
        let source = "let x = 4, y = 2; x++ / $262.gc() / y;";
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["gc"]
        );
    }

    #[test]
    fn base_and_unknown_properties_fail_closed_but_optional_hooks_do_not() {
        let source = "$262.global; $262.codePointRange; $262.futureHook();";
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["global", "unknown:$262.futureHook"]
        );
    }

    #[test]
    fn detach_harness_self_test_shadow_suppresses_the_include_hint() {
        let metadata = metadata(&[], &[], &["detachArrayBuffer.js"]);
        let source = "var /* intentional host shadow */ $262 = { detachArrayBuffer() {} };";
        assert!(
            missing_host_capability_hints(
                Path::new("test/harness/detachArrayBuffer-host-detachArrayBuffer.js"),
                source,
                &metadata,
                false,
            )
            .is_empty()
        );

        assert_eq!(
            missing_host_capability_hints(Path::new("test/ordinary.js"), source, &metadata, false,),
            ["detach-array-buffer"]
        );
    }

    #[test]
    fn installed_hosts_remove_only_their_typed_discovered_gaps() {
        let metadata = metadata(&["CanBlockIsFalse"], &[], &["detachArrayBuffer.js"]);
        let mut missing = missing_host_capability_hints(
            Path::new("test/example.js"),
            "$262.createRealm(); $262.detachArrayBuffer(buffer); $262.evalScript('0'); \
             $262.gc(); $262.global; $262.agent; $262.IsHTMLDDA;",
            &metadata,
            false,
        );
        HostCapabilities {
            agent: false,
            can_block_false: true,
            create_realm: true,
            detach_array_buffer: true,
            eval_script: true,
            gc: true,
            global: true,
            is_html_dda: true,
        }
        .retain_missing(&mut missing);
        assert_eq!(missing, ["agent"]);
    }

    #[test]
    fn disabled_typed_hosts_remain_missing() {
        let mut missing = vec![
            "can-block:false".to_owned(),
            "create-realm".to_owned(),
            "detach-array-buffer".to_owned(),
            "eval-script".to_owned(),
            "gc".to_owned(),
            "global".to_owned(),
        ];
        HostCapabilities::default().retain_missing(&mut missing);
        assert_eq!(
            missing,
            [
                "can-block:false",
                "create-realm",
                "detach-array-buffer",
                "eval-script",
                "gc",
                "global",
            ]
        );
    }

    #[test]
    fn atomics_cross_realm_metadata_gap_is_source_audited_and_exact() {
        const PATH: &str = "test/staging/sm/Atomics/cross-compartment.js";
        const SOURCE: &str = "const otherGlobal = $262.createRealm().global; const buffer = new \
                              otherGlobal.SharedArrayBuffer(4); Atomics.load(new \
                              otherGlobal.Int32Array(buffer), 0);";
        const SOURCE_SHA256: &str =
            "3cb79dbb8554f721f371c78cad9fe21234dc9b249f27e15e372abacdd014cb47";

        let mut hints = BTreeSet::from(["host-create-realm-required".to_owned()]);
        insert_atomics_cross_realm_feature_hints(
            &mut hints,
            Path::new(PATH),
            SOURCE,
            &source_tokens(SOURCE, false),
            PATH,
            SOURCE_SHA256,
        )
        .unwrap();
        assert_eq!(
            hints,
            BTreeSet::from([
                "Atomics".to_owned(),
                "SharedArrayBuffer".to_owned(),
                "host-create-realm-required".to_owned(),
            ])
        );
        assert_eq!(
            supplemental_feature_hints(Path::new("test/example.js"), SOURCE).unwrap(),
            ["host-create-realm-required"]
        );

        assert!(supplemental_feature_hints(Path::new(PATH), SOURCE).is_err());

        let shape_drift = "$262.createRealm(); Atomics.load;";
        let mut shape_hints = BTreeSet::from(["host-create-realm-required".to_owned()]);
        assert!(
            insert_atomics_cross_realm_feature_hints(
                &mut shape_hints,
                Path::new(PATH),
                shape_drift,
                &source_tokens(shape_drift, false),
                PATH,
                &source_sha256(shape_drift).unwrap(),
            )
            .is_err()
        );
        assert_eq!(
            shape_hints,
            BTreeSet::from(["host-create-realm-required".to_owned()])
        );
    }

    #[test]
    fn atomics_detached_buffers_requirement_is_path_and_source_hash_bound() {
        const PATH: &str = "test/staging/sm/Atomics/detached-buffers.js";
        const SOURCE: &str = "abc";
        const SOURCE_SHA256: &str =
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

        let mut hints = BTreeSet::new();
        insert_exact_source_feature_hint(
            &mut hints,
            Path::new(PATH),
            SOURCE,
            PATH,
            SOURCE_SHA256,
            "Atomics",
        )
        .unwrap();
        assert_eq!(hints, BTreeSet::from(["Atomics".to_owned()]));

        let mut wrong_path_hints = BTreeSet::new();
        insert_exact_source_feature_hint(
            &mut wrong_path_hints,
            Path::new("test/example.js"),
            SOURCE,
            PATH,
            SOURCE_SHA256,
            "Atomics",
        )
        .unwrap();
        assert!(wrong_path_hints.is_empty());

        let mut drifted_source_hints = BTreeSet::new();
        assert!(
            insert_exact_source_feature_hint(
                &mut drifted_source_hints,
                Path::new(PATH),
                "abd",
                PATH,
                SOURCE_SHA256,
                "Atomics",
            )
            .is_err()
        );
        assert!(drifted_source_hints.is_empty());
        assert!(supplemental_feature_hints(Path::new(PATH), SOURCE).is_err());
    }

    #[test]
    fn realm_host_admission_tags_are_source_scoped_and_ignore_hidden_text() {
        assert_eq!(
            supplemental_feature_hints(
                Path::new("test/example.js"),
                "$262.evalScript('0'); $262.createRealm();"
            )
            .unwrap(),
            ["host-create-realm-required", "host-eval-script-required"]
        );
        assert!(
            supplemental_feature_hints(
                Path::new("test/example.js"),
                r#""$262.createRealm"; /* $262.evalScript */ 0;"#
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn all_seven_required_hooks_have_explicit_capability_ids() {
        let source = r#"
            $262.agent;
            $262.createRealm;
            $262.evalScript;
            $262.detachArrayBuffer;
            $262.IsHTMLDDA;
            $262.gc;
            $262.AbstractModuleSource;
        "#;
        let actual = missing_host_capability_hints(
            Path::new("test/example.js"),
            source,
            &Metadata::default(),
            false,
        );
        assert_eq!(
            actual.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "abstract-module-source".to_owned(),
                "agent".to_owned(),
                "create-realm".to_owned(),
                "detach-array-buffer".to_owned(),
                "eval-script".to_owned(),
                "gc".to_owned(),
                "is-html-dda".to_owned(),
            ])
        );
    }
}
