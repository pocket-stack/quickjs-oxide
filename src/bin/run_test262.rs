//! Process-isolated Test262 runner for the pinned QuickJS compatibility suite.
//!
//! The metadata/configuration model follows QuickJS 2026-06-04
//! `run-test262.c`. Each runnable script variant runs in a fresh process so a
//! future engine crash or an already-possible infinite loop is reported
//! without taking down the coordinator.

#[path = "run_test262/capabilities.rs"]
mod capabilities;
#[path = "run_test262/config.rs"]
mod config;
#[path = "run_test262/execution.rs"]
mod execution;
#[path = "run_test262/metadata.rs"]
mod metadata;
#[path = "run_test262/report.rs"]
mod report;
#[path = "run_test262/requirements.rs"]
mod requirements;
#[path = "run_test262/scheduler.rs"]
mod scheduler;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use capabilities::OxideProfile;
use config::{parse_config, skip_reason, validate_config, validate_suite, verify_sha256};
use execution::{run_isolated_worker, run_worker};
use metadata::{Metadata, parse_metadata};
use report::{WorkerResult, report_row, write_report};
use requirements::{
    generator_destructuring_source_needs_async_guard, missing_host_capability_hints,
};
use scheduler::run_bounded;

const TEST262_COMMIT: &str = "5c8206929d81b2d3d727ca6aac56c18358c8d790";
const TEST262_PATCH_SHA256: &str =
    "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3";
const TEST262_CONFIG_SHA256: &str =
    "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b";
const TEST262_METADATA_SHA256: &str =
    "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a";
const TEST262_OXIDE_PROFILE_SHA256: &str =
    "3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef";
const TEST262_AGGREGATE_ERROR_PROFILE_SHA256: &str =
    "ad9e38f7b1b42445a848ee01437e925fc23f5525276bc45dd15c5ae7a1454d7a";
const TEST262_AGGREGATE_ERROR_MANIFEST_SHA256: &str =
    "f54979cc3881fd7d361dda7ffbbe75a5bf846e233512c7428711c1091b8474c5";
const TEST262_ARGUMENT_SPREAD_PROFILE_SHA256: &str =
    "5db27822923dd066c7afb448ae5dcdef25e57573cd2ac651dfe2b13892980112";
const TEST262_ARGUMENT_SPREAD_MANIFEST_SHA256: &str =
    "a8073747a0b8fea8fcdfe450766004ed9444bbc189ccfb52cf678556140ce184";
const TEST262_ASYNC_FUNCTION_CORE_PROFILE_SHA256: &str =
    "7fb94b8e350b5a270ab5f685f0a223e32c7d12fedf0ac3e0c1e157b03f4f0b33";
const TEST262_ASYNC_FUNCTION_CORE_MANIFEST_SHA256: &str =
    "97930e30959d8bdbdd1b030e4f4e94fe9657791951f48e58a6790e73a7191390";
const TEST262_ASYNC_GENERATOR_CORE_PROFILE_SHA256: &str =
    "edb34a6dd924e3b01535b94e24495ba69a4a195b7492fed670f17714d5e543d7";
const TEST262_ASYNC_GENERATOR_CORE_MANIFEST_SHA256: &str =
    "bfc4244e45d22fd2d98c06f6d413cc7e58b58b004dfc3eebcc7d964834108e9f";
const TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_PROFILE_SHA256: &str =
    "7c21b92bc769a6de2812f2c953bc7fe567e5df528255b4a85bfa429eb3d56ad9";
const TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_MANIFEST_SHA256: &str =
    "d4e3923053e589ec699880a946f5e1b9f00180c0b017a98377ed1a85643f3798";
const TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_PROFILE_SHA256: &str =
    "4c088b7e15be3bc1de099abf6560917c5677aa229fdc1799d0ff31367166ca63";
const TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_MANIFEST_SHA256: &str =
    "f7620c23730693b2b8b46ef85b2f373d9c5d0fd5c7da19b4af356ede77bcdc43";
const TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256: &str =
    "1b9d03b352d8e221cae6d0cc6c6c685776f16e0ca39c97c5fafc7b8bdca00f38";
const TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_MANIFEST_SHA256: &str =
    "82bae49d063b9691d245f1a08d0e37583fc27282ceb878cca7c4e1129e6fcad6";
const TEST262_ASYNC_GENERATOR_YIELD_STAR_PROFILE_SHA256: &str =
    "80bd7d1c042473a76ba15d85b3e5bbd6ebf175f0543c57e2908fd99a6b7b5256";
const TEST262_ASYNC_GENERATOR_YIELD_STAR_MANIFEST_SHA256: &str =
    "bb31f01a982136b336f9267701ef8b2874bc0596e226f6e9ca5b59e7b9af09fb";
const TEST262_FOR_AWAIT_OF_PROFILE_SHA256: &str =
    "d5d30d77eaabebeea1a9fa3cb18f555e3c5d69d263d1b82ca624c339f6262a2e";
const TEST262_FOR_AWAIT_OF_MANIFEST_SHA256: &str =
    "f87858a6c22df8c689d15f081075cba2758feb63eacb4be9ee310e72e9d17a0a";
const TEST262_ASYNC_ARROW_CORE_PROFILE_SHA256: &str =
    "f6634c6298e3d3fb740c0f55e8932ddc402ca8e120d8f0d2d9326f552186af2c";
const TEST262_ASYNC_ARROW_CORE_MANIFEST_SHA256: &str =
    "d4bc4b286b2da1b19949d56b614e1d1af110437285827fa4f4c6cb00dae1d969";
const TEST262_ASYNC_OBJECT_METHOD_CORE_PROFILE_SHA256: &str =
    "ec8be515bb6f68cb3226f1770b4ac73b66c013d5c27a74bcda974770546b9e9f";
const TEST262_ASYNC_OBJECT_METHOD_CORE_MANIFEST_SHA256: &str =
    "38b1fd3cc785923d4e98a28b8e8daf19777bf02630634753715abf7160c9d796";
const TEST262_ASYNC_CLASS_METHOD_CORE_PROFILE_SHA256: &str =
    "9dbf8b47dafbc6df98ae38a1c24c489fc530bf93bc5be7cd8d9efa0d86a3bd4c";
const TEST262_ASYNC_CLASS_METHOD_CORE_MANIFEST_SHA256: &str =
    "220fd2dd88cef8efb4ff92616f01bd28cfbf6c0e0527cd20cd14a0dbb15db524";
const TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256: &str =
    "668acc7b6b7de1345a1baa90d4f60fb67a2fa8beb018ab12a9bcd4cfba928b8e";
const TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_MANIFEST_SHA256: &str =
    "baa888fd5d5bea134123d563f8cc23a2ab483d6b0644c319c8dbc210b1a8d5bf";
const TEST262_CLASS_BASE_PROFILE_SHA256: &str =
    "df73a1ac299cce6ade0b0638f0a4c3322310aa2db8e15a28039f483328e69f00";
const TEST262_CLASS_BASE_MANIFEST_SHA256: &str =
    "0894fc15cf840a8897ad1b9243324c6312f28fd90e78cdafa377170d15b79f5f";
const TEST262_CLASS_DERIVED_PROFILE_SHA256: &str =
    "1aa167fef279273185060224bd8a65765283d95fe1e08986c5c4ea197657e160";
const TEST262_CLASS_DERIVED_MANIFEST_SHA256: &str =
    "c9c477104d7f538c4b3fa58a108171be866273bedf19825bedf682afc9d00366";
const TEST262_CLASS_SYNC_MATRIX_PROFILE_SHA256: &str =
    "de71fc1d3c675ed25dc54d43222a10c4f3d607c14cb4d43628d7a4587827a7ef";
const TEST262_CLASS_SYNC_MATRIX_MANIFEST_SHA256: &str =
    "40f038bdc52c762baf7f16ea885c98fc3d0afd033e56059717e8627086e14c78";
const TEST262_CLASS_PUBLIC_INIT_PROFILE_SHA256: &str =
    "f02524f9abedc00c6c84dc33367680bf31a30ae94604a5317a6690f603cbd7b1";
const TEST262_CLASS_PUBLIC_INIT_MANIFEST_SHA256: &str =
    "e06b14730f68fa17bee6ff648c806db4e730c5a1abb7ae32f2093b2274e070f3";
const TEST262_CLASS_PRIVATE_FIELDS_PROFILE_SHA256: &str =
    "c03c22a7ea0d767536c77f1720b5c87766b06759d8a42a6e7b9ec3069633ffa2";
const TEST262_CLASS_PRIVATE_FIELDS_MANIFEST_SHA256: &str =
    "c64f7f33e60c623976e0be920889d71984ac0899ffe23cb26a3b6d0f9089fe34";
const TEST262_CLASS_PRIVATE_METHODS_PROFILE_SHA256: &str =
    "76b0fcc5610e2ceee386469344fd727a8c359abe884befccec1ab435fed93315";
const TEST262_CLASS_PRIVATE_METHODS_MANIFEST_SHA256: &str =
    "af3047bf66c6477f34d4229b03493a2c4247cc3f6f2b5dc4bf26e40c3ed4c7b6";
const TEST262_CLASS_PRIVATE_ACCESSORS_PROFILE_SHA256: &str =
    "1040d156877d88f6aae651f90b8fae472a8a4054d21f49bbbf2162d280afd884";
const TEST262_CLASS_PRIVATE_ACCESSORS_MANIFEST_SHA256: &str =
    "f8d7b7cb065cf15bae4066ec0790d1c7f0da513b83c8166aef20b3ad7e024cf4";
const TEST262_CLASS_GENERATOR_METHODS_PROFILE_SHA256: &str =
    "eab79cc5f8ba041e93b7ea04bc391bed8fa249eaf5cbb11857d533fe27028c52";
const TEST262_CLASS_GENERATOR_METHODS_MANIFEST_SHA256: &str =
    "30857ac44aa29bf86925b72b14da28c9215fb3bc29f81fc6b950694fa0d70b0f";
const TEST262_CLASS_PRIVATE_GENERATOR_METHODS_PROFILE_SHA256: &str =
    "e3732db0b47608265f4f950c1c72929e782eb507597c5f0b336896e51874133e";
const TEST262_CLASS_PRIVATE_GENERATOR_METHODS_MANIFEST_SHA256: &str =
    "b7b2c71cab374f9bcc6754bd9a80506d273d2e135e3f66eb373f325c94d33685";
const TEST262_PROMISE_CONSTRUCTOR_JOBS_PROFILE_SHA256: &str =
    "f3a07d4c1c839b4d252ed65f8fb9cadc1862cd31280002caa4656d581007eb71";
const TEST262_PROMISE_CONSTRUCTOR_JOBS_MANIFEST_SHA256: &str =
    "6cd3564883d5c0e459872b835e19ee7bb8c7f13716824fa2617ca1e698d5ed25";
const TEST262_PROMISE_RACE_TRY_WITH_RESOLVERS_PROFILE_SHA256: &str =
    "8548d12a4d7f3141583b986c8e3ffcae4e1afb93476ae8a444f64b940bb44654";
const TEST262_PROMISE_RACE_TRY_WITH_RESOLVERS_MANIFEST_SHA256: &str =
    "be545aefd5f2029faae9745d859a43de176ec9865599a916f15ec465bf84d340";
const TEST262_PROMISE_FINALLY_PROFILE_SHA256: &str =
    "fa10d45a7ddd3924e9124cfc42239e296847223c6c9686beb2a8435e9c83bf04";
const TEST262_PROMISE_FINALLY_MANIFEST_SHA256: &str =
    "9c24a81143fc4d3dcaa8251a2ed98e381f07cb7969f30427a60e9ca931941464";
const TEST262_PROMISE_ALL_PROFILE_SHA256: &str =
    "83b69f80efbe0aa1c1273c646595424d4e3cda01f65ccc1e7400495a6779bb21";
const TEST262_PROMISE_ALL_MANIFEST_SHA256: &str =
    "293639a6d0e3f1937535997a4f61613fd40b2b10267d1d27cc5faa231865c1e5";
const TEST262_PROMISE_ALL_SETTLED_PROFILE_SHA256: &str =
    "755439ed09621a0460802bfda11ed27983364d572b13baaf93a2e00c5b481947";
const TEST262_PROMISE_ALL_SETTLED_MANIFEST_SHA256: &str =
    "5ac6c5f7e21194ee432a6480fc8e8b5ae7fff2c3c859aa61da98f7605261eb52";
const TEST262_PROMISE_ANY_PROFILE_SHA256: &str =
    "8059eea59f179846a4739ddb280b4d16518286261d1cdb361a2d383474f27826";
const TEST262_PROMISE_ANY_MANIFEST_SHA256: &str =
    "331a3d6f0b19a9353904afa5c5d740f844f97c89fcbc99b58cd11275d3b67eaf";
const TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256: &str =
    "8232e2c11e908f7cbf5a9e0f34fbd5223a9551b49ae64647f2a72b2314bcaf84";
const TEST262_ARRAY_BINDING_FLAT_MANIFEST_SHA256: &str =
    "db17670a1f7715a325a07087b766f6e64cf2bb24cec727278db05db3f79ee679";
const TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256: &str =
    "c770387473b6ba2e273ab635182b5f07ae80ad902f48057ba5e2fb4f036c723e";
const TEST262_ARRAY_BINDING_NESTED_MANIFEST_SHA256: &str =
    "f7c7c181cdde65c84dfcb677cbe45f77884990666a774f952bc165df89f5e8a5";
const TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256: &str =
    "b2133d90974566c72ab788525254de68d260b44756a8c5981111873fb38727af";
const TEST262_ARRAY_ASSIGNMENT_FLAT_MANIFEST_SHA256: &str =
    "046679bd745132066b4982770f13236bfecdbd953b70bdba98afa60424c599c8";
const TEST262_CATCH_BINDING_PROFILE_SHA256: &str =
    "a654327057a974e0feab6799f3c99a3104884a403cbc41bbc85f3fc226328718";
const TEST262_CATCH_BINDING_MANIFEST_SHA256: &str =
    "e3fb469169b069c185a7d9ea6b8cdce2fdb54d49181b7e87e33cff59a27c212e";
const TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256: &str =
    "5c98d19ccb72c7e2c577ddc98ee4ac83d43a0ba7d49175a8ebe271866d0feab6";
const TEST262_IDENTIFIER_DEFAULTS_MANIFEST_SHA256: &str =
    "264bb2b25e7502eed86f8a5df1b3fe8c0ccdeecd43171af390764b5e053a6472";
const TEST262_PARAMETER_DIRECT_EVAL_PROFILE_SHA256: &str =
    "98b5e323db1b4be493c1e05b8937a1060b71f7a1cc126087d05e88e7c2a2b335";
const TEST262_PARAMETER_DIRECT_EVAL_MANIFEST_SHA256: &str =
    "3df66805796888dd41acbc007b2a958aba5751e9694c0deffa5f0efba19c61a1";
const TEST262_PARAMETER_BINDING_PATTERNS_PROFILE_SHA256: &str =
    "1f25a0648044b6cb3027e23bc58032b2b2fc3517cd0a29b35d5e4d0844fc6e5e";
const TEST262_PARAMETER_BINDING_PATTERNS_MANIFEST_SHA256: &str =
    "9cb9662c3c5860e05ba2199be6d3818091e64780ccf7ef61c6d63276a6747f60";
const TEST262_PARAMETER_EXPRESSION_BINDING_PATTERNS_PROFILE_SHA256: &str =
    "0addc7345b6576e1944afc3d5d84cffe16e299e44af09245e78c08cb29207f7b";
const TEST262_PARAMETER_EXPRESSION_BINDING_PATTERNS_MANIFEST_SHA256: &str =
    "1db4662456a3ea231c7ce3f629d5224a8cb19d38d13d69c83e43f6407aac21c0";
const TEST262_IDENTIFIER_REST_PROFILE_SHA256: &str =
    "da6a76cb6338019f5c233e252bf6d40b7f3eb5c4235a6967cf78f9a74917dced";
const TEST262_IDENTIFIER_REST_MANIFEST_SHA256: &str =
    "cc326a73c13d2cd90726150e77ad5f5a247074f12a233fe9efa382b3ec6c420e";
const TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256: &str =
    "989f5617484d5c12a15fb26a447121fa3436b19f05cd998cf400b5d3d7179a51";
const TEST262_OBJECT_ASSIGNMENT_FLAT_MANIFEST_SHA256: &str =
    "92089af97dcc157d557061120dfdb68c868f2a8823288290a227a22bfadb285b";
const TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256: &str =
    "18411f3d674a9493806bbf6a601bda903e859395aeec572e466c4a59470ceb12";
const TEST262_OBJECT_ASSIGNMENT_NESTED_MANIFEST_SHA256: &str =
    "0e5a594cee6e1c021f310c8e9d88e8b253d789171c97511aec4adcfd346d7d27";
const TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256: &str =
    "4b9f50b982dc5c3af1466d425a1665448c4a00165d465a74fd4057ef6e414206";
const TEST262_OBJECT_ASSIGNMENT_REST_MANIFEST_SHA256: &str =
    "931d743e7e2f46d78e66baf7c7c83fcf33208fd8ced6f6c72619ec5948971226";
const TEST262_OBJECT_BINDING_PROFILE_SHA256: &str =
    "aa6cdca241b5f0be7eb202461ba80e44132f917a66480f1c04225cedc410d0d7";
const TEST262_OBJECT_BINDING_MANIFEST_SHA256: &str =
    "ab9974676a1f15442875d6b9de607a27a94a76896a949c8b9cf86b05dbac18dc";
const TEST262_OBJECT_REST_BINDING_PROFILE_SHA256: &str =
    "122a2b055aaf40672a0540441861ecd1e6c09b65e88d45b947bc27a691afc45e";
const TEST262_OBJECT_REST_BINDING_MANIFEST_SHA256: &str =
    "fc75564488d2ae45a015fa8b07989f3a178f08978221d87ffdeeca0a9359fe57";
const TEST262_OBJECT_REST_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "b51eee39825e3325effab1c326df30b999e636f67c8ce7bb800f0afdc2d8eab4";
const TEST262_OBJECT_REST_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "f229cd652dd5b38ed3a0387a089eab974148d404bd166e8b4c0eb2cb0fa7a2c1";
const TEST262_OBJECT_REST_GLOBAL_MANIFEST_SHA256: &str =
    "c0c20cc6d5bad2dd2f92b977497dacb62e77797af237c2a840c92247b60955cf";
const TEST262_OBJECT_REST_COMPANION_MANIFEST_SHA256: &str =
    "4effcae61bf4ca623de68d7a7e9eadb9e4c215d5c8e80b930c6a6f34a4eb7cfa";
const TEST262_ARRAY_BUFFER_PROFILE_SHA256: &str =
    "0803a027b2e9c238f80189993968816adfdda983ef3b23114a06f07b26c2d598";
const TEST262_ARRAY_BUFFER_MANIFEST_SHA256: &str =
    "d5720cc22c785d3757eb4e30aa3de53a664d58133a2323c6afe6233788014d01";
const TEST262_DATA_VIEW_PROFILE_SHA256: &str =
    "485ea3baf6695767108fb9f7f346c3a82d5a3db000af4510d6d002b313990cc8";
const TEST262_DATA_VIEW_MANIFEST_SHA256: &str =
    "3475b4a32f0a5f0ab50d5cd4e4843a7c7a59365298ecabcc5986b3fdd3f697e2";
const TEST262_DATA_VIEW_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "63f139b1a74da9a6114180593770dbcc86bb84fbafab5731f59e1387175c5a6a";
const TEST262_DATA_VIEW_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "b51eee39825e3325effab1c326df30b999e636f67c8ce7bb800f0afdc2d8eab4";
const TEST262_DATA_VIEW_GLOBAL_MANIFEST_SHA256: &str =
    "dc7c4e6d43ca6e86f4119b2f684fe1f8c538b2a07e598323a11edeb01e1f40cf";
const TEST262_TYPED_ARRAY_CORE_PROFILE_SHA256: &str =
    "dd106c074751866ce667352d3449cc0ec7d9b9072034a4f0a97050da7b7bad13";
const TEST262_TYPED_ARRAY_CORE_MANIFEST_SHA256: &str =
    "91ac9a132c8099ecd15d3cfcfe160b21a1f7e9a083a5210a33406606270ad378";
const TEST262_UINT8ARRAY_CODECS_PROFILE_SHA256: &str =
    "2e8f870a5c6d1c05adc37c759098d2412943beff8b8de3c1593ba74df7761ac9";
const TEST262_UINT8ARRAY_CODECS_MANIFEST_SHA256: &str =
    "2a52c3f54ef83a8df736e823d76e17927b670045f42d338d42a64f0e48681bb2";
const TEST262_UINT8ARRAY_CODECS_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9";
const TEST262_UINT8ARRAY_CODECS_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "ed80ab5aed86c606a1d7b5c1854b78ab1bb3c517cf0c6898a89e9f8d19135000";
const TEST262_RESIZABLE_ARRAYBUFFER_PROFILE_SHA256: &str =
    "e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898";
const TEST262_RESIZABLE_ARRAYBUFFER_MANIFEST_SHA256: &str =
    "f6e3b6ceb2e2b725a42543bf0f6652652ad4f0716657bd6ba62398cb7df38295";
const TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "ed80ab5aed86c606a1d7b5c1854b78ab1bb3c517cf0c6898a89e9f8d19135000";
const TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898";
const TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_MANIFEST_SHA256: &str =
    "5b58d035de75cc264f1fa3497458d25b5fd0c525b8a0eebe9838e34aff54ab1e";
const TEST262_COMPUTED_PROPERTY_NAMES_PARENT_PROFILE_SHA256: &str =
    "e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898";
const TEST262_COMPUTED_PROPERTY_NAMES_CANDIDATE_PROFILE_SHA256: &str =
    "fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a";
const TEST262_COMPUTED_PROPERTY_NAMES_MANIFEST_SHA256: &str =
    "478f57b13521b3e93df055cc43c44a14c197cbed65f3616b0dbe24ec87d9d5b5";
const TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898";
const TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a";
const TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_MANIFEST_SHA256: &str =
    "478f57b13521b3e93df055cc43c44a14c197cbed65f3616b0dbe24ec87d9d5b5";
const TEST262_REST_PARAMETERS_PARENT_PROFILE_SHA256: &str =
    "fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a";
const TEST262_REST_PARAMETERS_CANDIDATE_PROFILE_SHA256: &str =
    "d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969";
const TEST262_REST_PARAMETERS_MANIFEST_SHA256: &str =
    "2757425b53ce1f046c5c4a063b3931c9299c64f8c8911a764a83ff720407ad46";
const TEST262_DEFAULT_PARAMETERS_PARENT_PROFILE_SHA256: &str =
    "d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969";
const TEST262_DEFAULT_PARAMETERS_CANDIDATE_PROFILE_SHA256: &str =
    "9c345c1e2d79911eec5d6c8750a730f3b3ed0dbefdcd483e0f9c92fcf66aeca0";
const TEST262_DEFAULT_PARAMETERS_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "63f139b1a74da9a6114180593770dbcc86bb84fbafab5731f59e1387175c5a6a";
const TEST262_DEFAULT_PARAMETERS_MANIFEST_SHA256: &str =
    "b61ac2d12fffe88b77fa1edec117a795390de9bdf16ee65509393461bc7b2cff";
const TEST262_DEFAULT_PARAMETERS_STRICT_BODY_MANIFEST_SHA256: &str =
    "1d85b3a86d471a5a3f814f9a8c6ba2e34a89d9815ac08a92c37b26d45ea2bbcd";
const TEST262_MAP_PROFILE_SHA256: &str =
    "16ab6bfe18540aae398c847905f492491e81500045b45a6bfb21f447fd537ea2";
const TEST262_MAP_MANIFEST_SHA256: &str =
    "f369837ef69275815349f9202ade5b6ae1d4d91e9ae0313ac816ecfb0e3a4845";
const TEST262_SET_PROFILE_SHA256: &str =
    "6869e9d28fff1d5bd4e5b698dcdf6ee677b9134a91781ad7abe226200d669455";
const TEST262_SET_MANIFEST_SHA256: &str =
    "0f560c202e9463ff4896796be6e924db984e25bc3e95ae2604a54ce9dee61e9f";
const TEST262_WEAK_COLLECTIONS_PROFILE_SHA256: &str =
    "a23cfb3270eb40eb3839413f3dacaf75fee2cecaca9d1b0ecc40d2c6c3c804c1";
const TEST262_WEAK_COLLECTIONS_MANIFEST_SHA256: &str =
    "6189cde88a7fcb15222d536d19f3e8172be66e35de24f47107e0c67910b92b7a";
const TEST262_WEAK_COLLECTIONS_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "f229cd652dd5b38ed3a0387a089eab974148d404bd166e8b4c0eb2cb0fa7a2c1";
const TEST262_WEAK_COLLECTIONS_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef";
const TEST262_WEAK_COLLECTIONS_GLOBAL_MANIFEST_SHA256: &str =
    "d0bd5c21db1165cd72618168ce5592b78a6909be5f2cd0813fa15ed6a3c17cc1";
const TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256: &str =
    "ff674aafc4b1b61b0c40042f831b44c600b1f741e06b8c8c35863b876919aa7b";
const TEST262_SYMBOL_PROTOCOLS_MANIFEST_SHA256: &str =
    "6147636f7950b899f7c0eea25078e2f4c9c4c7fda2977181dd7c9671aa0bcde2";
const TEST262_GENERATOR_DESTRUCTURING_PROFILE_SHA256: &str =
    "8057ef347c07ffc80a66c5c83ff73873148a8813af49bcca1ced9863cfb9ac9e";
const TEST262_GENERATOR_DESTRUCTURING_MANIFEST_SHA256: &str =
    "07ad2748c65763366ebdcb8c01893a13aa4fbbcca3e900a31042fc670593f3c5";
const TEST262_ITERATOR_HELPERS_PROFILE_SHA256: &str =
    "a0ed7fa1a5cd46c5c47895d671c0078434635ae41f0a420e66573dcb86d18a7f";
const TEST262_ITERATOR_HELPERS_MANIFEST_SHA256: &str =
    "6db8a38003ba95245dde0e0559b64a75c1a0215e610408811174f482363b729c";
const TEST262_ITERATOR_HELPERS_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "205554c5686ef2ec77420984ce038d321411a11acabefd2c37d9b63b67fcba62";
const TEST262_ITERATOR_HELPERS_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910";
const TEST262_ITERATOR_HELPERS_GLOBAL_MANIFEST_SHA256: &str =
    "c4700fe6efcfa05d4e00c3d7cfc9d4a4aa062db7ac58cd8318a51bf41c1bbcf4";
const TEST262_GLOBAL_THIS_PARENT_PROFILE_SHA256: &str =
    "8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910";
const TEST262_GLOBAL_THIS_CANDIDATE_PROFILE_SHA256: &str =
    "caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15";
const TEST262_GLOBAL_THIS_ACTIVATION_MANIFEST_SHA256: &str =
    "4d8be634488c72eafbbd350f0d75829f4d3f71fb4b141db192e5f69ace41ea23";
const TEST262_GLOBAL_THIS_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910";
const TEST262_GLOBAL_THIS_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15";
const TEST262_GLOBAL_THIS_GLOBAL_MANIFEST_SHA256: &str =
    "aecc6d30cc47676fd20541c509c1016b3cd8d238e96afa6178d3f0c2bd62abc4";
const TEST262_PROMISE_GLOBAL_PARENT_PROFILE_SHA256: &str =
    "caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15";
const TEST262_PROMISE_GLOBAL_CANDIDATE_PROFILE_SHA256: &str =
    "5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9";
const TEST262_PROMISE_GLOBAL_MANIFEST_SHA256: &str =
    "1d1016e310a423629b8be481912823c0d1f7c078dd21710f01fc0350d6f589ba";
const TEST262_ITERATOR_SEQUENCING_PROFILE_SHA256: &str =
    "8284db009a398fb88b2d357d7d8255479943d963574392f7b718610ee12cb16a";
const TEST262_ITERATOR_SEQUENCING_MANIFEST_SHA256: &str =
    "74eebb8c63a2606e54e1d0023c5244b8a0538ac51d1ca0a105fe56a04fa74af2";
const TEST262_OPTIONAL_CHAINING_PROFILE_SHA256: &str =
    "42bdcf4005aafed999604c10db1298651875210ea2ee2d96569a3ec54a99e064";
const TEST262_OPTIONAL_CHAINING_MANIFEST_SHA256: &str =
    "c49c346272b7910aee065ccfc866439b8acce2c656919198631ab55ce4316c45";
const TEST262_PROXY_PROFILE_SHA256: &str =
    "0c151608ed8cd580238e27188f5e63382ee11e1dc91f7c480db2c366e1232d12";
const TEST262_PROXY_MANIFEST_SHA256: &str =
    "ef2395cd3bd268d7ba1010773651408826452feaed121f8f2d4c0e6afeed66f3";
const TEST262_REGEXP_BUILTINS_PROFILE_SHA256: &str =
    "0214f6789a3276c4755fadde19477b70620184a6137d29eefef0975cfb379c15";
const TEST262_REGEXP_BUILTINS_MANIFEST_SHA256: &str =
    "db6201093f57412de0d0cf16d4ff06f74512af3bc76d6f83c337474c7b982ab3";
const QUICKJS_VERSION: &str = "2026-06-04";
const DEFAULT_TIMEOUT_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Variant {
    Sloppy,
    Strict,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::Sloppy => "sloppy",
            Self::Strict => "strict",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "sloppy" => Ok(Self::Sloppy),
            "strict" => Ok(Self::Strict),
            _ => Err(format!("unknown Test262 variant: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestMode {
    DefaultSloppy,
    DefaultStrict,
    Sloppy,
    Strict,
    Both,
}

impl TestMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default" | "default-sloppy" | "default-nostrict" => Ok(Self::DefaultSloppy),
            "default-strict" => Ok(Self::DefaultStrict),
            "sloppy" | "nostrict" => Ok(Self::Sloppy),
            "strict" => Ok(Self::Strict),
            "both" | "all" => Ok(Self::Both),
            _ => Err(format!("unknown Test262 mode: {value}")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DefaultSloppy => "default-sloppy",
            Self::DefaultStrict => "default-strict",
            Self::Sloppy => "sloppy",
            Self::Strict => "strict",
            Self::Both => "both",
        }
    }
}

#[derive(Clone, Debug)]
struct CoordinatorOptions {
    suite: PathBuf,
    config: PathBuf,
    oxide_profile: PathBuf,
    manifest: Option<PathBuf>,
    tests: Vec<PathBuf>,
    all: bool,
    report: PathBuf,
    mode: TestMode,
    timeout: Duration,
    workers: usize,
    allow_failures: bool,
}

#[derive(Clone, Debug)]
struct WorkerOptions {
    suite: PathBuf,
    test: PathBuf,
    variant: Variant,
    allow_async_host: bool,
}

#[derive(Clone, Debug)]
struct MetadataAuditOptions {
    suite: PathBuf,
    records: PathBuf,
}

enum Invocation {
    Coordinator(CoordinatorOptions),
    Worker(WorkerOptions),
    MetadataAudit(MetadataAuditOptions),
    Help,
}

fn main() -> ExitCode {
    let invocation = match parse_args(env::args_os().skip(1)) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("run-test262: {error}");
            eprintln!("run-test262: use --help for usage");
            return ExitCode::from(2);
        }
    };
    match invocation {
        Invocation::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Invocation::Worker(options) => match run_worker(&options) {
            Ok(result) => {
                println!("{}", result.encode());
                ExitCode::SUCCESS
            }
            Err(error) => {
                println!(
                    "{}",
                    WorkerResult::failure("runner-error", "host", "", error).encode()
                );
                ExitCode::SUCCESS
            }
        },
        Invocation::MetadataAudit(options) => match audit_metadata(&options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("run-test262 metadata audit: {error}");
                ExitCode::from(2)
            }
        },
        Invocation::Coordinator(options) => match run_coordinator(&options) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(error) => {
                eprintln!("run-test262: {error}");
                ExitCode::from(2)
            }
        },
    }
}

fn parse_args(arguments: impl Iterator<Item = OsString>) -> Result<Invocation, String> {
    let arguments = arguments
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return Ok(Invocation::Help);
    }

    let worker = arguments.iter().any(|argument| argument == "--worker-one");
    let mut suite = None;
    let mut config = None;
    let mut oxide_profile = None;
    let mut manifest = None;
    let mut tests = Vec::new();
    let mut report = None;
    let mut mode = TestMode::Both;
    let mut mode_explicit = false;
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    let mut timeout_explicit = false;
    let mut workers = None;
    let mut all = false;
    let mut allow_failures = false;
    let mut variant = None;
    let mut metadata_records = None;
    let mut allow_async_host = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        index += 1;
        let mut take_value = |name: &str| -> Result<String, String> {
            let value = arguments
                .get(index)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))?;
            index += 1;
            Ok(value)
        };
        match argument.as_str() {
            "--worker-one" => {}
            "--allow-async-host" => allow_async_host = true,
            "--suite" => suite = Some(PathBuf::from(take_value("--suite")?)),
            "--config" => config = Some(PathBuf::from(take_value("--config")?)),
            "--oxide-profile" => {
                oxide_profile = Some(PathBuf::from(take_value("--oxide-profile")?));
            }
            "--manifest" => manifest = Some(PathBuf::from(take_value("--manifest")?)),
            "--test" => tests.push(PathBuf::from(take_value("--test")?)),
            "--report" => report = Some(PathBuf::from(take_value("--report")?)),
            "--mode" => {
                mode = TestMode::parse(&take_value("--mode")?)?;
                mode_explicit = true;
            }
            "--variant" => variant = Some(Variant::parse(&take_value("--variant")?)?),
            "--timeout-ms" => {
                timeout_explicit = true;
                timeout_ms = take_value("--timeout-ms")?
                    .parse::<u64>()
                    .map_err(|_| "--timeout-ms must be an unsigned integer".to_owned())?;
                if timeout_ms == 0 {
                    return Err("--timeout-ms must be greater than zero".to_owned());
                }
            }
            "--workers" => {
                let value = take_value("--workers")?
                    .parse::<usize>()
                    .map_err(|_| "--workers must be a positive integer".to_owned())?;
                if value == 0 {
                    return Err("--workers must be greater than zero".to_owned());
                }
                workers = Some(value);
            }
            "--all" => all = true,
            "--allow-failures" => allow_failures = true,
            "--validate-metadata" => {
                metadata_records = Some(PathBuf::from(take_value("--validate-metadata")?));
            }
            unknown => return Err(format!("unknown option: {unknown}")),
        }
    }

    let suite = suite.ok_or_else(|| "--suite is required".to_owned())?;
    if let Some(records) = metadata_records {
        if worker
            || all
            || manifest.is_some()
            || !tests.is_empty()
            || report.is_some()
            || config.is_some()
            || oxide_profile.is_some()
            || variant.is_some()
            || allow_failures
            || mode_explicit
            || timeout_explicit
            || workers.is_some()
            || allow_async_host
        {
            return Err("--validate-metadata cannot be combined with execution options".to_owned());
        }
        return Ok(Invocation::MetadataAudit(MetadataAuditOptions {
            suite,
            records,
        }));
    }
    if worker {
        if all
            || manifest.is_some()
            || tests.len() != 1
            || report.is_some()
            || config.is_some()
            || oxide_profile.is_some()
            || allow_failures
            || mode_explicit
            || timeout_explicit
            || workers.is_some()
        {
            return Err("invalid coordinator option passed to --worker-one".to_owned());
        }
        return Ok(Invocation::Worker(WorkerOptions {
            suite,
            test: tests.remove(0),
            variant: variant.ok_or_else(|| "--worker-one requires --variant".to_owned())?,
            allow_async_host,
        }));
    }
    if allow_async_host {
        return Err("--allow-async-host is internal to --worker-one".to_owned());
    }
    if variant.is_some() {
        return Err("--variant is internal to --worker-one".to_owned());
    }
    let input_count =
        usize::from(all) + usize::from(manifest.is_some()) + usize::from(!tests.is_empty());
    if input_count != 1 {
        return Err("select exactly one of --all, --manifest, or one-or-more --test".to_owned());
    }
    let config = config.unwrap_or_else(|| {
        suite
            .parent()
            .unwrap_or(Path::new("."))
            .join("test262.conf")
    });
    let oxide_profile = oxide_profile.ok_or_else(|| "--oxide-profile is required".to_owned())?;
    let report = report.ok_or_else(|| "--report is required".to_owned())?;
    Ok(Invocation::Coordinator(CoordinatorOptions {
        suite,
        config,
        oxide_profile,
        manifest,
        tests,
        all,
        report,
        mode,
        timeout: Duration::from_millis(timeout_ms),
        workers: workers.unwrap_or_else(default_worker_count),
        allow_failures,
    }))
}

fn print_help() {
    let default_workers = default_worker_count();
    println!(
        "run-test262 (quickjs-oxide)\n\
usage: run-test262 --suite DIR --config FILE --oxide-profile FILE (--manifest FILE | --test FILE... | --all) --report FILE [options]\n\
\n\
  --mode MODE          both, strict, sloppy, default-strict, or default-sloppy\n\
  --timeout-ms N       hard per-variant worker timeout (default: {DEFAULT_TIMEOUT_MS})\n\
  --workers N          maximum concurrent subprocesses (default: {default_workers})\n\
  --allow-failures     record a baseline without returning a failing status\n\
  --validate-metadata FILE\n\
                       serialize the complete pinned metadata inventory\n\
\n\
Every variant runs in a fresh subprocess. Module tests remain unsupported;\n\
async tests remain fail-closed unless a pinned scoped profile opts into the\n\
job-draining Test262 host."
    );
}

fn default_worker_count() -> usize {
    let available = thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let quickjs_style = if available >= 8 {
        available - 1
    } else {
        available
    };
    quickjs_style.clamp(1, 16)
}

struct PlannedTest {
    relative: PathBuf,
    metadata: Metadata,
}

struct PlannedRow {
    test_index: usize,
    variant: Option<Variant>,
    result: Option<WorkerResult>,
}

#[derive(Clone, Copy)]
struct RunnableJob {
    row_index: usize,
    test_index: usize,
    variant: Variant,
}

fn run_coordinator(options: &CoordinatorOptions) -> Result<bool, String> {
    validate_suite(&options.suite)?;
    validate_config(&options.config)?;
    let oxide_profile_sha256 = verify_oxide_profile(options)?;
    let config = parse_config(&options.config)?;
    let oxide_profile = OxideProfile::load(&options.oxide_profile)?;
    validate_oxide_profile(&oxide_profile, &options.suite)?;
    let harness_dir = config
        .harness_dir
        .clone()
        .unwrap_or_else(|| options.suite.join("harness"));
    if !harness_dir.is_dir() {
        return Err(format!(
            "harness directory is missing: {}",
            harness_dir.display()
        ));
    }
    let actual_harness = fs::canonicalize(&harness_dir)
        .map_err(|error| format!("resolve {}: {error}", harness_dir.display()))?;
    let suite_harness = fs::canonicalize(options.suite.join("harness"))
        .map_err(|error| format!("resolve suite harness: {error}"))?;
    if actual_harness != suite_harness {
        return Err(format!(
            "pinned config harness does not match the suite harness: {}",
            harness_dir.display()
        ));
    }
    let tests = collect_tests(options)?;
    let executable = env::current_exe().map_err(|error| format!("locate runner: {error}"))?;
    let mut planned_tests = Vec::with_capacity(tests.len());
    let mut planned_rows = Vec::new();
    let mut runnable_jobs = Vec::new();

    for relative in tests {
        let path = options.suite.join(&relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse metadata for {}: {error}", relative.display()))?;
        let variants = metadata.variants(options.mode);
        let skip = skip_reason(&relative, &metadata, &config);
        let mut missing_host = missing_host_capability_hints(
            &relative,
            &source,
            &metadata,
            oxide_profile.allows_async_execution(),
        );
        execution::WORKER_HOST_CAPABILITIES.retain_missing(&mut missing_host);
        let capability =
            oxide_profile.classify(&relative, &metadata.features, metadata.negative.is_some());
        let selection_result = if let Some((outcome, detail)) = &skip {
            Some(WorkerResult::failure(outcome, "selection", "", detail))
        } else if let Some(result) = missing_host_result(&missing_host) {
            Some(result)
        } else if let Some(classification) = capability {
            Some(WorkerResult::failure(
                classification.outcome,
                "selection",
                "EngineCapability",
                classification.detail,
            ))
        } else if !oxide_profile.allows_async_execution()
            && generator_destructuring_source_needs_async_guard(&source, &metadata)
        {
            Some(WorkerResult::failure(
                "unsupported-async",
                "selection",
                "ExecutionMode",
                "missing execution capabilities: async",
            ))
        } else {
            None
        };
        let test_index = planned_tests.len();
        planned_tests.push(PlannedTest { relative, metadata });

        if variants.is_empty() {
            planned_rows.push(PlannedRow {
                test_index,
                variant: None,
                result: Some(WorkerResult::failure(
                    "skipped-mode",
                    "selection",
                    "",
                    "variant excluded by mode",
                )),
            });
            continue;
        }

        for variant in variants {
            let row_index = planned_rows.len();
            planned_rows.push(PlannedRow {
                test_index,
                variant: Some(variant),
                result: selection_result.clone(),
            });
            if selection_result.is_none() {
                runnable_jobs.push(RunnableJob {
                    row_index,
                    test_index,
                    variant,
                });
            }
        }
    }

    let worker_results = run_bounded(runnable_jobs.len(), options.workers, |job_index| {
        let job = runnable_jobs[job_index];
        let test = &planned_tests[job.test_index];
        run_isolated_worker(
            &executable,
            &options.suite,
            &test.relative,
            job.variant,
            options.timeout,
            oxide_profile.allows_async_execution(),
        )
    })?;
    for (job, result) in runnable_jobs.iter().zip(worker_results) {
        planned_rows[job.row_index].result = Some(result);
    }

    let mut rows = Vec::with_capacity(planned_rows.len());
    let mut summary = BTreeMap::<String, usize>::new();
    for row in planned_rows {
        let test = &planned_tests[row.test_index];
        let result = row
            .result
            .ok_or_else(|| format!("missing result for {}", test.relative.display()))?;
        *summary.entry(result.outcome.clone()).or_default() += 1;
        rows.push(report_row(
            &test.relative,
            row.variant.map_or("none", Variant::name),
            &test.metadata,
            &result,
        ));
    }

    write_report(options, &rows, &summary, oxide_profile_sha256)?;
    let total = rows.len();
    let passed = summary.get("pass").copied().unwrap_or(0);
    let skipped = summary
        .iter()
        .filter(|(name, _)| name.starts_with("skipped-"))
        .map(|(_, count)| *count)
        .sum::<usize>();
    let unsupported = summary
        .iter()
        .filter(|(name, _)| name.starts_with("unsupported-"))
        .map(|(_, count)| *count)
        .sum::<usize>();
    let failed = total.saturating_sub(passed + skipped);
    println!(
        "Test262: total={total} pass={passed} fail={failed} unsupported={unsupported} skipped={skipped}"
    );
    println!(
        "execution: runnable={} workers={}",
        runnable_jobs.len(),
        options.workers.min(runnable_jobs.len())
    );
    println!("report={}", options.report.display());
    Ok(options.allow_failures || failed == 0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OxideProfileKind {
    Global,
    AggregateError,
    ArgumentSpread,
    AsyncFunctionCore,
    AsyncGeneratorCore,
    AsyncGeneratorObjectMethodCore,
    AsyncGeneratorClassMethodCore,
    AsyncGeneratorPrivateClassMethodCore,
    AsyncGeneratorYieldStar,
    ForAwaitOf,
    AsyncArrowCore,
    AsyncObjectMethodCore,
    AsyncClassMethodCore,
    AsyncPrivateClassMethodCore,
    ClassBase,
    ClassDerived,
    ClassSyncMatrix,
    ClassPublicInit,
    ClassPrivateFields,
    ClassPrivateMethods,
    ClassPrivateAccessors,
    ClassGeneratorMethods,
    ClassPrivateGeneratorMethods,
    PromiseConstructorJobs,
    PromiseRaceTryWithResolvers,
    PromiseFinally,
    PromiseAll,
    PromiseAllSettled,
    PromiseAny,
    ArrayBindingFlat,
    ArrayBindingNested,
    ArrayAssignmentFlat,
    CatchBinding,
    IdentifierDefaults,
    ParameterDirectEval,
    ParameterBindingPatterns,
    ParameterExpressionBindingPatterns,
    IdentifierRest,
    ObjectAssignmentFlat,
    ObjectAssignmentNested,
    ObjectAssignmentRest,
    ObjectBinding,
    ObjectRestBinding,
    ObjectRestGlobalParent,
    ObjectRestGlobalCandidate,
    ArrayBuffer,
    DataView,
    DataViewGlobalParent,
    DataViewGlobalCandidate,
    TypedArrayCore,
    Uint8ArrayCodecs,
    Uint8ArrayCodecsGlobalParent,
    Uint8ArrayCodecsGlobalCandidate,
    ResizableArrayBuffer,
    ResizableArrayBufferGlobalParent,
    ResizableArrayBufferGlobalCandidate,
    ComputedPropertyNamesParent,
    ComputedPropertyNamesCandidate,
    ComputedPropertyNamesGlobalParent,
    ComputedPropertyNamesGlobalCandidate,
    RestParametersParent,
    RestParametersCandidate,
    DefaultParametersParent,
    DefaultParametersCandidate,
    DefaultParametersGlobalCandidate,
    Map,
    Set,
    WeakCollections,
    WeakCollectionsGlobalParent,
    WeakCollectionsGlobalCandidate,
    SymbolProtocols,
    GeneratorDestructuring,
    IteratorHelpers,
    IteratorHelpersGlobalParent,
    IteratorHelpersGlobalCandidate,
    GlobalThisParent,
    GlobalThisCandidate,
    GlobalThisGlobalParent,
    GlobalThisGlobalCandidate,
    PromiseGlobalParent,
    PromiseGlobalCandidate,
    IteratorSequencing,
    OptionalChaining,
    Proxy,
    RegExpBuiltins,
}

fn identify_oxide_profile(path: &Path) -> Result<OxideProfileKind, String> {
    let actual = fs::canonicalize(path).map_err(|error| {
        format!(
            "resolve Test262 capability profile {}: {error}",
            path.display()
        )
    })?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profiles = [
        (
            root.join("compat/test262-oxide.conf"),
            OxideProfileKind::Global,
        ),
        (
            root.join("tests/test262-aggregate-error.conf"),
            OxideProfileKind::AggregateError,
        ),
        (
            root.join("tests/test262-argument-spread.conf"),
            OxideProfileKind::ArgumentSpread,
        ),
        (
            root.join("tests/test262-async-function-core.conf"),
            OxideProfileKind::AsyncFunctionCore,
        ),
        (
            root.join("tests/test262-async-generator-core.conf"),
            OxideProfileKind::AsyncGeneratorCore,
        ),
        (
            root.join("tests/test262-async-generator-object-method-core.conf"),
            OxideProfileKind::AsyncGeneratorObjectMethodCore,
        ),
        (
            root.join("tests/test262-async-generator-class-method-core.conf"),
            OxideProfileKind::AsyncGeneratorClassMethodCore,
        ),
        (
            root.join("tests/test262-async-generator-private-class-method-core.conf"),
            OxideProfileKind::AsyncGeneratorPrivateClassMethodCore,
        ),
        (
            root.join("tests/test262-async-generator-yield-star.conf"),
            OxideProfileKind::AsyncGeneratorYieldStar,
        ),
        (
            root.join("compat/test262-for-await-of.conf"),
            OxideProfileKind::ForAwaitOf,
        ),
        (
            root.join("tests/test262-async-arrow-core.conf"),
            OxideProfileKind::AsyncArrowCore,
        ),
        (
            root.join("tests/test262-async-object-method-core.conf"),
            OxideProfileKind::AsyncObjectMethodCore,
        ),
        (
            root.join("tests/test262-async-class-method-core.conf"),
            OxideProfileKind::AsyncClassMethodCore,
        ),
        (
            root.join("tests/test262-async-private-class-method-core.conf"),
            OxideProfileKind::AsyncPrivateClassMethodCore,
        ),
        (
            root.join("tests/test262-class-base.conf"),
            OxideProfileKind::ClassBase,
        ),
        (
            root.join("tests/test262-class-derived.conf"),
            OxideProfileKind::ClassDerived,
        ),
        (
            root.join("tests/test262-class-sync-matrix.conf"),
            OxideProfileKind::ClassSyncMatrix,
        ),
        (
            root.join("tests/test262-class-public-init.conf"),
            OxideProfileKind::ClassPublicInit,
        ),
        (
            root.join("tests/test262-class-private-fields.conf"),
            OxideProfileKind::ClassPrivateFields,
        ),
        (
            root.join("tests/test262-class-private-methods.conf"),
            OxideProfileKind::ClassPrivateMethods,
        ),
        (
            root.join("tests/test262-class-private-accessors.conf"),
            OxideProfileKind::ClassPrivateAccessors,
        ),
        (
            root.join("tests/test262-class-generator-methods.conf"),
            OxideProfileKind::ClassGeneratorMethods,
        ),
        (
            root.join("tests/test262-class-private-generator-methods.conf"),
            OxideProfileKind::ClassPrivateGeneratorMethods,
        ),
        (
            root.join("tests/test262-promise-constructor-jobs.conf"),
            OxideProfileKind::PromiseConstructorJobs,
        ),
        (
            root.join("tests/test262-promise-race-try-with-resolvers.conf"),
            OxideProfileKind::PromiseRaceTryWithResolvers,
        ),
        (
            root.join("tests/test262-promise-finally.conf"),
            OxideProfileKind::PromiseFinally,
        ),
        (
            root.join("tests/test262-promise-all.conf"),
            OxideProfileKind::PromiseAll,
        ),
        (
            root.join("tests/test262-promise-all-settled.conf"),
            OxideProfileKind::PromiseAllSettled,
        ),
        (
            root.join("tests/test262-promise-any.conf"),
            OxideProfileKind::PromiseAny,
        ),
        (
            root.join("tests/test262-array-binding-flat.conf"),
            OxideProfileKind::ArrayBindingFlat,
        ),
        (
            root.join("tests/test262-array-binding-nested.conf"),
            OxideProfileKind::ArrayBindingNested,
        ),
        (
            root.join("tests/test262-array-assignment-flat.conf"),
            OxideProfileKind::ArrayAssignmentFlat,
        ),
        (
            root.join("tests/test262-catch-binding.conf"),
            OxideProfileKind::CatchBinding,
        ),
        (
            root.join("tests/test262-identifier-defaults.conf"),
            OxideProfileKind::IdentifierDefaults,
        ),
        (
            root.join("tests/test262-parameter-direct-eval.conf"),
            OxideProfileKind::ParameterDirectEval,
        ),
        (
            root.join("tests/test262-parameter-binding-patterns.conf"),
            OxideProfileKind::ParameterBindingPatterns,
        ),
        (
            root.join("tests/test262-parameter-expression-binding-patterns.conf"),
            OxideProfileKind::ParameterExpressionBindingPatterns,
        ),
        (
            root.join("tests/test262-identifier-rest.conf"),
            OxideProfileKind::IdentifierRest,
        ),
        (
            root.join("tests/test262-object-assignment-flat.conf"),
            OxideProfileKind::ObjectAssignmentFlat,
        ),
        (
            root.join("tests/test262-object-assignment-nested.conf"),
            OxideProfileKind::ObjectAssignmentNested,
        ),
        (
            root.join("tests/test262-object-assignment-rest.conf"),
            OxideProfileKind::ObjectAssignmentRest,
        ),
        (
            root.join("tests/test262-object-binding.conf"),
            OxideProfileKind::ObjectBinding,
        ),
        (
            root.join("tests/test262-object-rest-binding.conf"),
            OxideProfileKind::ObjectRestBinding,
        ),
        (
            root.join("tests/test262-object-rest-global-parent.conf"),
            OxideProfileKind::ObjectRestGlobalParent,
        ),
        (
            root.join("tests/test262-object-rest-global-candidate.conf"),
            OxideProfileKind::ObjectRestGlobalCandidate,
        ),
        (
            root.join("tests/test262-array-buffer.conf"),
            OxideProfileKind::ArrayBuffer,
        ),
        (
            root.join("tests/test262-data-view.conf"),
            OxideProfileKind::DataView,
        ),
        (
            root.join("tests/test262-data-view-global-parent.conf"),
            OxideProfileKind::DataViewGlobalParent,
        ),
        (
            root.join("tests/test262-data-view-global-candidate.conf"),
            OxideProfileKind::DataViewGlobalCandidate,
        ),
        (
            root.join("tests/test262-typed-array-core.conf"),
            OxideProfileKind::TypedArrayCore,
        ),
        (
            root.join("tests/test262-uint8array-codecs.conf"),
            OxideProfileKind::Uint8ArrayCodecs,
        ),
        (
            root.join("tests/test262-uint8array-codecs-global-parent.conf"),
            OxideProfileKind::Uint8ArrayCodecsGlobalParent,
        ),
        (
            root.join("tests/test262-uint8array-codecs-global-candidate.conf"),
            OxideProfileKind::Uint8ArrayCodecsGlobalCandidate,
        ),
        (
            root.join("tests/test262-resizable-arraybuffer.conf"),
            OxideProfileKind::ResizableArrayBuffer,
        ),
        (
            root.join("tests/test262-resizable-arraybuffer-global-parent.conf"),
            OxideProfileKind::ResizableArrayBufferGlobalParent,
        ),
        (
            root.join("tests/test262-resizable-arraybuffer-global-candidate.conf"),
            OxideProfileKind::ResizableArrayBufferGlobalCandidate,
        ),
        (
            root.join("tests/test262-computed-property-names-parent.conf"),
            OxideProfileKind::ComputedPropertyNamesParent,
        ),
        (
            root.join("tests/test262-computed-property-names.conf"),
            OxideProfileKind::ComputedPropertyNamesCandidate,
        ),
        (
            root.join("tests/test262-computed-property-names-global-parent.conf"),
            OxideProfileKind::ComputedPropertyNamesGlobalParent,
        ),
        (
            root.join("tests/test262-computed-property-names-global-candidate.conf"),
            OxideProfileKind::ComputedPropertyNamesGlobalCandidate,
        ),
        (
            root.join("tests/test262-rest-parameters-parent.conf"),
            OxideProfileKind::RestParametersParent,
        ),
        (
            root.join("tests/test262-rest-parameters-candidate.conf"),
            OxideProfileKind::RestParametersCandidate,
        ),
        (
            root.join("tests/test262-default-parameters-parent.conf"),
            OxideProfileKind::DefaultParametersParent,
        ),
        (
            root.join("tests/test262-default-parameters-candidate.conf"),
            OxideProfileKind::DefaultParametersCandidate,
        ),
        (
            root.join("tests/test262-default-parameters-global-candidate.conf"),
            OxideProfileKind::DefaultParametersGlobalCandidate,
        ),
        (root.join("tests/test262-map.conf"), OxideProfileKind::Map),
        (root.join("tests/test262-set.conf"), OxideProfileKind::Set),
        (
            root.join("tests/test262-weak-collections.conf"),
            OxideProfileKind::WeakCollections,
        ),
        (
            root.join("tests/test262-weak-collections-global-parent.conf"),
            OxideProfileKind::WeakCollectionsGlobalParent,
        ),
        (
            root.join("tests/test262-weak-collections-global-candidate.conf"),
            OxideProfileKind::WeakCollectionsGlobalCandidate,
        ),
        (
            root.join("tests/test262-symbol-protocols.conf"),
            OxideProfileKind::SymbolProtocols,
        ),
        (
            root.join("tests/test262-generator-destructuring.conf"),
            OxideProfileKind::GeneratorDestructuring,
        ),
        (
            root.join("tests/test262-iterator-helpers.conf"),
            OxideProfileKind::IteratorHelpers,
        ),
        (
            root.join("tests/test262-iterator-helpers-global-parent.conf"),
            OxideProfileKind::IteratorHelpersGlobalParent,
        ),
        (
            root.join("tests/test262-iterator-helpers-global-candidate.conf"),
            OxideProfileKind::IteratorHelpersGlobalCandidate,
        ),
        (
            root.join("tests/test262-global-this-parent.conf"),
            OxideProfileKind::GlobalThisParent,
        ),
        (
            root.join("tests/test262-global-this-candidate.conf"),
            OxideProfileKind::GlobalThisCandidate,
        ),
        (
            root.join("tests/test262-global-this-global-parent.conf"),
            OxideProfileKind::GlobalThisGlobalParent,
        ),
        (
            root.join("tests/test262-global-this-global-candidate.conf"),
            OxideProfileKind::GlobalThisGlobalCandidate,
        ),
        (
            root.join("tests/test262-promise-global-parent.conf"),
            OxideProfileKind::PromiseGlobalParent,
        ),
        (
            root.join("tests/test262-promise-global-candidate.conf"),
            OxideProfileKind::PromiseGlobalCandidate,
        ),
        (
            root.join("tests/test262-iterator-sequencing.conf"),
            OxideProfileKind::IteratorSequencing,
        ),
        (
            root.join("tests/test262-optional-chaining.conf"),
            OxideProfileKind::OptionalChaining,
        ),
        (
            root.join("tests/test262-proxy.conf"),
            OxideProfileKind::Proxy,
        ),
        (
            root.join("tests/test262-regexp-builtins.conf"),
            OxideProfileKind::RegExpBuiltins,
        ),
    ];
    for (candidate, kind) in profiles {
        let candidate = fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "resolve pinned Test262 capability profile {}: {error}",
                candidate.display()
            )
        })?;
        if actual == candidate {
            return Ok(kind);
        }
    }
    Err(format!(
        "unsupported Test262 capability profile: {}; expected compat/test262-oxide.conf or a pinned tests/test262-*.conf profile",
        path.display()
    ))
}

fn verify_scoped_pinned_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
    manifest_relative: &str,
    manifest_sha256: &str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("scoped {label} Test262 capability profile"),
    )?;
    if options.all || !options.tests.is_empty() {
        return Err(format!(
            "the scoped {label} Test262 capability profile requires its pinned manifest"
        ));
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!("the scoped {label} Test262 capability profile requires its pinned manifest")
    })?;
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve scoped {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| format!("resolve pinned scoped {label} manifest: {error}"))?;
    if actual != expected {
        return Err(format!(
            "the scoped {label} Test262 capability profile requires {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        manifest_sha256,
        &format!("scoped {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_scoped_derived_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
    manifest_sha256: &str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("scoped {label} Test262 capability profile"),
    )?;
    if options.all || !options.tests.is_empty() {
        return Err(format!(
            "the scoped {label} Test262 capability profile requires its authenticated manifest"
        ));
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!("the scoped {label} Test262 capability profile requires its authenticated manifest")
    })?;
    verify_sha256(
        manifest,
        manifest_sha256,
        &format!("scoped {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_historical_global_transition_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("historical global transition {label} Test262 capability profile"),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the historical global transition {label} Test262 capability profile requires --all or its pinned manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the historical global transition {label} Test262 capability profile requires --all or its pinned manifest"
        )
    })?;
    let manifest_relative = "tests/test262-iterator-helpers-global.txt";
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve historical global transition {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| {
            format!("resolve pinned historical global transition {label} manifest: {error}")
        })?;
    if actual != expected {
        return Err(format!(
            "the historical global transition {label} Test262 capability profile requires --all or {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        TEST262_ITERATOR_HELPERS_GLOBAL_MANIFEST_SHA256,
        &format!("historical global transition {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_global_this_global_transition_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("historical globalThis transition {label} Test262 capability profile"),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the historical globalThis transition {label} Test262 capability profile requires --all or its pinned tag manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the historical globalThis transition {label} Test262 capability profile requires --all or its pinned tag manifest"
        )
    })?;
    let manifest_relative = "tests/test262-global-this.txt";
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve historical globalThis transition {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| {
            format!("resolve pinned historical globalThis transition {label} manifest: {error}")
        })?;
    if actual != expected {
        return Err(format!(
            "the historical globalThis transition {label} Test262 capability profile requires --all or {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        TEST262_GLOBAL_THIS_GLOBAL_MANIFEST_SHA256,
        &format!("historical globalThis transition {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_promise_global_transition_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("historical Promise transition {label} Test262 capability profile"),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the historical Promise transition {label} Test262 capability profile requires --all or its pinned tag manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the historical Promise transition {label} Test262 capability profile requires --all or its pinned tag manifest"
        )
    })?;
    let manifest_relative = "tests/test262-promise-global.txt";
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve historical Promise transition {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| {
            format!("resolve pinned historical Promise transition {label} manifest: {error}")
        })?;
    if actual != expected {
        return Err(format!(
            "the historical Promise transition {label} Test262 capability profile requires --all or {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        TEST262_PROMISE_GLOBAL_MANIFEST_SHA256,
        &format!("historical Promise transition {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_uint8array_codecs_global_transition_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("historical Uint8Array codec transition {label} Test262 capability profile"),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the historical Uint8Array codec transition {label} Test262 capability profile requires --all or its pinned tag manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the historical Uint8Array codec transition {label} Test262 capability profile requires --all or its pinned tag manifest"
        )
    })?;
    let manifest_relative = "tests/test262-uint8array-codecs.txt";
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve historical Uint8Array codec transition {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| {
            format!(
                "resolve pinned historical Uint8Array codec transition {label} manifest: {error}"
            )
        })?;
    if actual != expected {
        return Err(format!(
            "the historical Uint8Array codec transition {label} Test262 capability profile requires --all or {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        TEST262_UINT8ARRAY_CODECS_MANIFEST_SHA256,
        &format!("historical Uint8Array codec transition {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_resizable_arraybuffer_global_transition_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("historical resizable-arraybuffer transition {label} Test262 capability profile"),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the historical resizable-arraybuffer transition {label} Test262 capability profile requires --all or its pinned tag-universe manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the historical resizable-arraybuffer transition {label} Test262 capability profile requires --all or its pinned tag-universe manifest"
        )
    })?;
    let manifest_relative = "tests/test262-resizable-arraybuffer-universe.txt";
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve historical resizable-arraybuffer transition {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| {
            format!(
                "resolve pinned historical resizable-arraybuffer transition {label} manifest: {error}"
            )
        })?;
    if actual != expected {
        return Err(format!(
            "the historical resizable-arraybuffer transition {label} Test262 capability profile requires --all or {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_MANIFEST_SHA256,
        &format!("historical resizable-arraybuffer transition {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_computed_property_names_global_transition_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!(
            "historical computed-property-names transition {label} Test262 capability profile"
        ),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the historical computed-property-names transition {label} Test262 capability profile requires --all or its pinned tag-universe manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the historical computed-property-names transition {label} Test262 capability profile requires --all or its pinned tag-universe manifest"
        )
    })?;
    let manifest_relative = "tests/test262-computed-property-names-universe.txt";
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve historical computed-property-names transition {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| {
            format!(
                "resolve pinned historical computed-property-names transition {label} manifest: {error}"
            )
        })?;
    if actual != expected {
        return Err(format!(
            "the historical computed-property-names transition {label} Test262 capability profile requires --all or {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_MANIFEST_SHA256,
        &format!("historical computed-property-names transition {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_tag_transition_profile(
    options: &CoordinatorOptions,
    cohort: &str,
    label: &str,
    profile_sha256: &'static str,
    manifests: &[(&str, &str)],
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("{cohort} {label} Test262 capability profile"),
    )?;
    if !options.tests.is_empty() {
        return Err(format!(
            "the {cohort} {label} Test262 capability profile requires --all or its pinned tag-universe manifest"
        ));
    }
    if options.all {
        return Ok(profile_sha256);
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the {cohort} {label} Test262 capability profile requires --all or its pinned tag-universe manifest"
        )
    })?;
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve {cohort} {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    for (manifest_relative, manifest_sha256) in manifests {
        let expected =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
                .map_err(|error| {
                    format!("resolve pinned {cohort} {label} manifest {manifest_relative}: {error}")
                })?;
        if actual == expected {
            verify_sha256(
                manifest,
                manifest_sha256,
                &format!("{cohort} {label} Test262 manifest"),
            )?;
            return Ok(profile_sha256);
        }
    }
    let allowed = manifests
        .iter()
        .map(|(relative, _)| *relative)
        .collect::<Vec<_>>()
        .join(" or ");
    Err(format!(
        "the {cohort} {label} Test262 capability profile requires --all or {allowed}, found {}",
        manifest.display()
    ))
}

fn verify_scoped_object_assignment_profile(
    options: &CoordinatorOptions,
    cohort: &str,
    profile_sha256: &'static str,
    manifest_sha256: &str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("scoped {cohort} object assignment Test262 capability profile"),
    )?;
    if options.all || !options.tests.is_empty() {
        return Err(format!(
            "the scoped {cohort} object assignment Test262 capability profile requires its pinned manifest"
        ));
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the scoped {cohort} object assignment Test262 capability profile requires its pinned manifest"
        )
    })?;
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve scoped {cohort} object assignment manifest {}: {error}",
            manifest.display()
        )
    })?;
    let relative = format!("tests/test262-object-assignment-{cohort}.txt");
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(&relative))
        .map_err(|error| {
            format!("resolve pinned scoped {cohort} object assignment manifest: {error}")
        })?;
    if actual != expected {
        return Err(format!(
            "the scoped {cohort} object assignment Test262 capability profile requires {relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        manifest_sha256,
        &format!("scoped {cohort} object assignment Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_oxide_profile(options: &CoordinatorOptions) -> Result<&'static str, String> {
    match identify_oxide_profile(&options.oxide_profile)? {
        OxideProfileKind::Global => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_OXIDE_PROFILE_SHA256,
                "global quickjs-oxide Test262 capability profile",
            )?;
            Ok(TEST262_OXIDE_PROFILE_SHA256)
        }
        OxideProfileKind::AggregateError => verify_scoped_pinned_profile(
            options,
            "AggregateError",
            TEST262_AGGREGATE_ERROR_PROFILE_SHA256,
            "tests/test262-aggregate-error.txt",
            TEST262_AGGREGATE_ERROR_MANIFEST_SHA256,
        ),
        OxideProfileKind::ArgumentSpread => verify_scoped_pinned_profile(
            options,
            "argument spread",
            TEST262_ARGUMENT_SPREAD_PROFILE_SHA256,
            "tests/test262-argument-spread.txt",
            TEST262_ARGUMENT_SPREAD_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncFunctionCore => verify_scoped_pinned_profile(
            options,
            "ordinary async function core",
            TEST262_ASYNC_FUNCTION_CORE_PROFILE_SHA256,
            "tests/test262-async-function-core.txt",
            TEST262_ASYNC_FUNCTION_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncGeneratorCore => verify_scoped_pinned_profile(
            options,
            "ordinary async-generator function core",
            TEST262_ASYNC_GENERATOR_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-core.txt",
            TEST262_ASYNC_GENERATOR_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncGeneratorObjectMethodCore => verify_scoped_pinned_profile(
            options,
            "async-generator object method core",
            TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-object-method-core.txt",
            TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncGeneratorClassMethodCore => verify_scoped_pinned_profile(
            options,
            "public async-generator class method core",
            TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-class-method-core.txt",
            TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncGeneratorPrivateClassMethodCore => verify_scoped_pinned_profile(
            options,
            "private async-generator class method core",
            TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-private-class-method-core.txt",
            TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncGeneratorYieldStar => verify_scoped_pinned_profile(
            options,
            "async-generator yield-star",
            TEST262_ASYNC_GENERATOR_YIELD_STAR_PROFILE_SHA256,
            "tests/test262-async-generator-yield-star.txt",
            TEST262_ASYNC_GENERATOR_YIELD_STAR_MANIFEST_SHA256,
        ),
        OxideProfileKind::ForAwaitOf => verify_scoped_derived_profile(
            options,
            "for-await-of",
            TEST262_FOR_AWAIT_OF_PROFILE_SHA256,
            TEST262_FOR_AWAIT_OF_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncArrowCore => verify_scoped_pinned_profile(
            options,
            "async arrow core",
            TEST262_ASYNC_ARROW_CORE_PROFILE_SHA256,
            "tests/test262-async-arrow-core.txt",
            TEST262_ASYNC_ARROW_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncObjectMethodCore => verify_scoped_pinned_profile(
            options,
            "ordinary async object method core",
            TEST262_ASYNC_OBJECT_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-object-method-core.txt",
            TEST262_ASYNC_OBJECT_METHOD_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncClassMethodCore => verify_scoped_pinned_profile(
            options,
            "public ordinary async class method core",
            TEST262_ASYNC_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-class-method-core.txt",
            TEST262_ASYNC_CLASS_METHOD_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::AsyncPrivateClassMethodCore => verify_scoped_pinned_profile(
            options,
            "ordinary private async class method core",
            TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-private-class-method-core.txt",
            TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassBase => verify_scoped_pinned_profile(
            options,
            "base class",
            TEST262_CLASS_BASE_PROFILE_SHA256,
            "tests/test262-class-base.txt",
            TEST262_CLASS_BASE_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassDerived => verify_scoped_pinned_profile(
            options,
            "derived class",
            TEST262_CLASS_DERIVED_PROFILE_SHA256,
            "tests/test262-class-derived.txt",
            TEST262_CLASS_DERIVED_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassSyncMatrix => verify_scoped_pinned_profile(
            options,
            "class sync matrix",
            TEST262_CLASS_SYNC_MATRIX_PROFILE_SHA256,
            "tests/test262-class-sync-matrix.txt",
            TEST262_CLASS_SYNC_MATRIX_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassPublicInit => verify_scoped_pinned_profile(
            options,
            "public class initialization",
            TEST262_CLASS_PUBLIC_INIT_PROFILE_SHA256,
            "tests/test262-class-public-init.txt",
            TEST262_CLASS_PUBLIC_INIT_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassPrivateFields => verify_scoped_pinned_profile(
            options,
            "private class fields",
            TEST262_CLASS_PRIVATE_FIELDS_PROFILE_SHA256,
            "tests/test262-class-private-fields.txt",
            TEST262_CLASS_PRIVATE_FIELDS_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassPrivateMethods => verify_scoped_pinned_profile(
            options,
            "private class methods",
            TEST262_CLASS_PRIVATE_METHODS_PROFILE_SHA256,
            "tests/test262-class-private-methods.txt",
            TEST262_CLASS_PRIVATE_METHODS_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassPrivateAccessors => verify_scoped_pinned_profile(
            options,
            "private class accessors",
            TEST262_CLASS_PRIVATE_ACCESSORS_PROFILE_SHA256,
            "tests/test262-class-private-accessors.txt",
            TEST262_CLASS_PRIVATE_ACCESSORS_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassGeneratorMethods => verify_scoped_pinned_profile(
            options,
            "class generator methods",
            TEST262_CLASS_GENERATOR_METHODS_PROFILE_SHA256,
            "tests/test262-class-generator-methods.txt",
            TEST262_CLASS_GENERATOR_METHODS_MANIFEST_SHA256,
        ),
        OxideProfileKind::ClassPrivateGeneratorMethods => verify_scoped_pinned_profile(
            options,
            "private class generator methods",
            TEST262_CLASS_PRIVATE_GENERATOR_METHODS_PROFILE_SHA256,
            "tests/test262-class-private-generator-methods.txt",
            TEST262_CLASS_PRIVATE_GENERATOR_METHODS_MANIFEST_SHA256,
        ),
        OxideProfileKind::PromiseConstructorJobs => verify_scoped_pinned_profile(
            options,
            "Promise constructor and jobs",
            TEST262_PROMISE_CONSTRUCTOR_JOBS_PROFILE_SHA256,
            "tests/test262-promise-constructor-jobs.txt",
            TEST262_PROMISE_CONSTRUCTOR_JOBS_MANIFEST_SHA256,
        ),
        OxideProfileKind::PromiseRaceTryWithResolvers => verify_scoped_pinned_profile(
            options,
            "Promise race, try, and withResolvers",
            TEST262_PROMISE_RACE_TRY_WITH_RESOLVERS_PROFILE_SHA256,
            "tests/test262-promise-race-try-with-resolvers.txt",
            TEST262_PROMISE_RACE_TRY_WITH_RESOLVERS_MANIFEST_SHA256,
        ),
        OxideProfileKind::PromiseFinally => verify_scoped_pinned_profile(
            options,
            "Promise finally",
            TEST262_PROMISE_FINALLY_PROFILE_SHA256,
            "tests/test262-promise-finally.txt",
            TEST262_PROMISE_FINALLY_MANIFEST_SHA256,
        ),
        OxideProfileKind::PromiseAll => verify_scoped_pinned_profile(
            options,
            "Promise all",
            TEST262_PROMISE_ALL_PROFILE_SHA256,
            "tests/test262-promise-all.txt",
            TEST262_PROMISE_ALL_MANIFEST_SHA256,
        ),
        OxideProfileKind::PromiseAllSettled => verify_scoped_pinned_profile(
            options,
            "Promise allSettled",
            TEST262_PROMISE_ALL_SETTLED_PROFILE_SHA256,
            "tests/test262-promise-all-settled.txt",
            TEST262_PROMISE_ALL_SETTLED_MANIFEST_SHA256,
        ),
        OxideProfileKind::PromiseAny => verify_scoped_pinned_profile(
            options,
            "Promise any",
            TEST262_PROMISE_ANY_PROFILE_SHA256,
            "tests/test262-promise-any.txt",
            TEST262_PROMISE_ANY_MANIFEST_SHA256,
        ),
        OxideProfileKind::ArrayBindingFlat => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256,
                "scoped flat array binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped flat array binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped flat array binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped flat array binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-array-binding-flat.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped flat array binding manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped flat array binding Test262 capability profile requires tests/test262-array-binding-flat.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_ARRAY_BINDING_FLAT_MANIFEST_SHA256,
                "scoped flat array binding Test262 manifest",
            )?;
            Ok(TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256)
        }
        OxideProfileKind::ArrayBindingNested => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256,
                "scoped nested array binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped nested array binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped nested array binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped nested array binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/test262-array-binding-nested.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped nested array binding manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped nested array binding Test262 capability profile requires tests/test262-array-binding-nested.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_ARRAY_BINDING_NESTED_MANIFEST_SHA256,
                "scoped nested array binding Test262 manifest",
            )?;
            Ok(TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256)
        }
        OxideProfileKind::ArrayAssignmentFlat => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256,
                "scoped flat array assignment Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped flat array assignment Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped flat array assignment Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped flat array assignment manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/test262-array-assignment-flat.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped flat array assignment manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped flat array assignment Test262 capability profile requires tests/test262-array-assignment-flat.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_ARRAY_ASSIGNMENT_FLAT_MANIFEST_SHA256,
                "scoped flat array assignment Test262 manifest",
            )?;
            Ok(TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256)
        }
        OxideProfileKind::CatchBinding => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_CATCH_BINDING_PROFILE_SHA256,
                "scoped catch binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped catch binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped catch binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped catch binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-catch-binding.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped catch binding manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped catch binding Test262 capability profile requires tests/test262-catch-binding.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_CATCH_BINDING_MANIFEST_SHA256,
                "scoped catch binding Test262 manifest",
            )?;
            Ok(TEST262_CATCH_BINDING_PROFILE_SHA256)
        }
        OxideProfileKind::IdentifierDefaults => verify_scoped_pinned_profile(
            options,
            "identifier defaults",
            TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256,
            "tests/test262-identifier-defaults.txt",
            TEST262_IDENTIFIER_DEFAULTS_MANIFEST_SHA256,
        ),
        OxideProfileKind::ParameterDirectEval => verify_scoped_pinned_profile(
            options,
            "parameter direct eval",
            TEST262_PARAMETER_DIRECT_EVAL_PROFILE_SHA256,
            "tests/test262-parameter-direct-eval.txt",
            TEST262_PARAMETER_DIRECT_EVAL_MANIFEST_SHA256,
        ),
        OxideProfileKind::ParameterBindingPatterns => verify_scoped_pinned_profile(
            options,
            "parameter BindingPatterns",
            TEST262_PARAMETER_BINDING_PATTERNS_PROFILE_SHA256,
            "tests/test262-parameter-binding-patterns.txt",
            TEST262_PARAMETER_BINDING_PATTERNS_MANIFEST_SHA256,
        ),
        OxideProfileKind::ParameterExpressionBindingPatterns => verify_scoped_pinned_profile(
            options,
            "parameter-expression BindingPatterns",
            TEST262_PARAMETER_EXPRESSION_BINDING_PATTERNS_PROFILE_SHA256,
            "tests/test262-parameter-expression-binding-patterns.txt",
            TEST262_PARAMETER_EXPRESSION_BINDING_PATTERNS_MANIFEST_SHA256,
        ),
        OxideProfileKind::IdentifierRest => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_IDENTIFIER_REST_PROFILE_SHA256,
                "scoped identifier rest Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped identifier rest Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped identifier rest Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped identifier rest manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-identifier-rest.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped identifier rest manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped identifier rest Test262 capability profile requires tests/test262-identifier-rest.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_IDENTIFIER_REST_MANIFEST_SHA256,
                "scoped identifier rest Test262 manifest",
            )?;
            Ok(TEST262_IDENTIFIER_REST_PROFILE_SHA256)
        }
        OxideProfileKind::ObjectAssignmentFlat => verify_scoped_object_assignment_profile(
            options,
            "flat",
            TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256,
            TEST262_OBJECT_ASSIGNMENT_FLAT_MANIFEST_SHA256,
        ),
        OxideProfileKind::ObjectAssignmentNested => verify_scoped_object_assignment_profile(
            options,
            "nested",
            TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256,
            TEST262_OBJECT_ASSIGNMENT_NESTED_MANIFEST_SHA256,
        ),
        OxideProfileKind::ObjectAssignmentRest => verify_scoped_object_assignment_profile(
            options,
            "rest",
            TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256,
            TEST262_OBJECT_ASSIGNMENT_REST_MANIFEST_SHA256,
        ),
        OxideProfileKind::ObjectBinding => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_OBJECT_BINDING_PROFILE_SHA256,
                "scoped object binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped object binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped object binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped object binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-object-binding.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped object binding manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped object binding Test262 capability profile requires tests/test262-object-binding.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_OBJECT_BINDING_MANIFEST_SHA256,
                "scoped object binding Test262 manifest",
            )?;
            Ok(TEST262_OBJECT_BINDING_PROFILE_SHA256)
        }
        OxideProfileKind::ObjectRestBinding => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_OBJECT_REST_BINDING_PROFILE_SHA256,
                "scoped object-rest binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped object-rest binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped object-rest binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped object-rest binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-object-rest-binding.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped object-rest binding manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped object-rest binding Test262 capability profile requires tests/test262-object-rest-binding.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_OBJECT_REST_BINDING_MANIFEST_SHA256,
                "scoped object-rest binding Test262 manifest",
            )?;
            Ok(TEST262_OBJECT_REST_BINDING_PROFILE_SHA256)
        }
        OxideProfileKind::ObjectRestGlobalParent => verify_tag_transition_profile(
            options,
            "object-rest global admission",
            "parent",
            TEST262_OBJECT_REST_GLOBAL_PARENT_PROFILE_SHA256,
            &[
                (
                    "tests/test262-object-rest-universe.txt",
                    TEST262_OBJECT_REST_GLOBAL_MANIFEST_SHA256,
                ),
                (
                    "tests/test262-object-rest-companion.txt",
                    TEST262_OBJECT_REST_COMPANION_MANIFEST_SHA256,
                ),
            ],
        ),
        OxideProfileKind::ObjectRestGlobalCandidate => verify_tag_transition_profile(
            options,
            "object-rest global admission",
            "candidate",
            TEST262_OBJECT_REST_GLOBAL_CANDIDATE_PROFILE_SHA256,
            &[
                (
                    "tests/test262-object-rest-universe.txt",
                    TEST262_OBJECT_REST_GLOBAL_MANIFEST_SHA256,
                ),
                (
                    "tests/test262-object-rest-companion.txt",
                    TEST262_OBJECT_REST_COMPANION_MANIFEST_SHA256,
                ),
            ],
        ),
        OxideProfileKind::ArrayBuffer => verify_scoped_pinned_profile(
            options,
            "ArrayBuffer",
            TEST262_ARRAY_BUFFER_PROFILE_SHA256,
            "tests/test262-array-buffer.txt",
            TEST262_ARRAY_BUFFER_MANIFEST_SHA256,
        ),
        OxideProfileKind::DataView => verify_scoped_pinned_profile(
            options,
            "DataView",
            TEST262_DATA_VIEW_PROFILE_SHA256,
            "tests/test262-data-view.txt",
            TEST262_DATA_VIEW_MANIFEST_SHA256,
        ),
        OxideProfileKind::DataViewGlobalParent => verify_tag_transition_profile(
            options,
            "DataView global admission",
            "parent",
            TEST262_DATA_VIEW_GLOBAL_PARENT_PROFILE_SHA256,
            &[(
                "tests/test262-data-view-universe.txt",
                TEST262_DATA_VIEW_GLOBAL_MANIFEST_SHA256,
            )],
        ),
        OxideProfileKind::DataViewGlobalCandidate => verify_tag_transition_profile(
            options,
            "DataView global admission",
            "candidate",
            TEST262_DATA_VIEW_GLOBAL_CANDIDATE_PROFILE_SHA256,
            &[(
                "tests/test262-data-view-universe.txt",
                TEST262_DATA_VIEW_GLOBAL_MANIFEST_SHA256,
            )],
        ),
        OxideProfileKind::TypedArrayCore => verify_scoped_pinned_profile(
            options,
            "TypedArray core",
            TEST262_TYPED_ARRAY_CORE_PROFILE_SHA256,
            "tests/test262-typed-array-core.txt",
            TEST262_TYPED_ARRAY_CORE_MANIFEST_SHA256,
        ),
        OxideProfileKind::Uint8ArrayCodecs => verify_scoped_pinned_profile(
            options,
            "Uint8Array base64 and hexadecimal codecs",
            TEST262_UINT8ARRAY_CODECS_PROFILE_SHA256,
            "tests/test262-uint8array-codecs.txt",
            TEST262_UINT8ARRAY_CODECS_MANIFEST_SHA256,
        ),
        OxideProfileKind::Uint8ArrayCodecsGlobalParent => {
            verify_uint8array_codecs_global_transition_profile(
                options,
                "pre-R3bs global parent",
                TEST262_UINT8ARRAY_CODECS_GLOBAL_PARENT_PROFILE_SHA256,
            )
        }
        OxideProfileKind::Uint8ArrayCodecsGlobalCandidate => {
            verify_uint8array_codecs_global_transition_profile(
                options,
                "R3bs global candidate",
                TEST262_UINT8ARRAY_CODECS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            )
        }
        OxideProfileKind::ResizableArrayBuffer => verify_scoped_pinned_profile(
            options,
            "resizable ArrayBuffer activation",
            TEST262_RESIZABLE_ARRAYBUFFER_PROFILE_SHA256,
            "tests/test262-resizable-arraybuffer.txt",
            TEST262_RESIZABLE_ARRAYBUFFER_MANIFEST_SHA256,
        ),
        OxideProfileKind::ResizableArrayBufferGlobalParent => {
            verify_resizable_arraybuffer_global_transition_profile(
                options,
                "pre-admission global parent",
                TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_PARENT_PROFILE_SHA256,
            )
        }
        OxideProfileKind::ResizableArrayBufferGlobalCandidate => {
            verify_resizable_arraybuffer_global_transition_profile(
                options,
                "resizable ArrayBuffer global candidate",
                TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_CANDIDATE_PROFILE_SHA256,
            )
        }
        OxideProfileKind::ComputedPropertyNamesParent => verify_scoped_pinned_profile(
            options,
            "computed property names parent",
            TEST262_COMPUTED_PROPERTY_NAMES_PARENT_PROFILE_SHA256,
            "tests/test262-computed-property-names-universe.txt",
            TEST262_COMPUTED_PROPERTY_NAMES_MANIFEST_SHA256,
        ),
        OxideProfileKind::ComputedPropertyNamesCandidate => verify_scoped_pinned_profile(
            options,
            "computed property names candidate",
            TEST262_COMPUTED_PROPERTY_NAMES_CANDIDATE_PROFILE_SHA256,
            "tests/test262-computed-property-names-universe.txt",
            TEST262_COMPUTED_PROPERTY_NAMES_MANIFEST_SHA256,
        ),
        OxideProfileKind::ComputedPropertyNamesGlobalParent => {
            verify_computed_property_names_global_transition_profile(
                options,
                "pre-admission global parent",
                TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_PARENT_PROFILE_SHA256,
            )
        }
        OxideProfileKind::ComputedPropertyNamesGlobalCandidate => {
            verify_computed_property_names_global_transition_profile(
                options,
                "computed property names global candidate",
                TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_CANDIDATE_PROFILE_SHA256,
            )
        }
        OxideProfileKind::RestParametersParent => verify_tag_transition_profile(
            options,
            "rest-parameters",
            "parent",
            TEST262_REST_PARAMETERS_PARENT_PROFILE_SHA256,
            &[(
                "tests/test262-rest-parameters-universe.txt",
                TEST262_REST_PARAMETERS_MANIFEST_SHA256,
            )],
        ),
        OxideProfileKind::RestParametersCandidate => verify_tag_transition_profile(
            options,
            "rest-parameters",
            "candidate",
            TEST262_REST_PARAMETERS_CANDIDATE_PROFILE_SHA256,
            &[(
                "tests/test262-rest-parameters-universe.txt",
                TEST262_REST_PARAMETERS_MANIFEST_SHA256,
            )],
        ),
        OxideProfileKind::DefaultParametersParent => verify_tag_transition_profile(
            options,
            "default-parameters",
            "parent",
            TEST262_DEFAULT_PARAMETERS_PARENT_PROFILE_SHA256,
            &[
                (
                    "tests/test262-default-parameters-universe.txt",
                    TEST262_DEFAULT_PARAMETERS_MANIFEST_SHA256,
                ),
                (
                    "tests/test262-default-parameters-strict-body.txt",
                    TEST262_DEFAULT_PARAMETERS_STRICT_BODY_MANIFEST_SHA256,
                ),
            ],
        ),
        OxideProfileKind::DefaultParametersCandidate => verify_tag_transition_profile(
            options,
            "default-parameters",
            "candidate",
            TEST262_DEFAULT_PARAMETERS_CANDIDATE_PROFILE_SHA256,
            &[
                (
                    "tests/test262-default-parameters-universe.txt",
                    TEST262_DEFAULT_PARAMETERS_MANIFEST_SHA256,
                ),
                (
                    "tests/test262-default-parameters-strict-body.txt",
                    TEST262_DEFAULT_PARAMETERS_STRICT_BODY_MANIFEST_SHA256,
                ),
            ],
        ),
        OxideProfileKind::DefaultParametersGlobalCandidate => verify_tag_transition_profile(
            options,
            "default-parameters global admission",
            "candidate",
            TEST262_DEFAULT_PARAMETERS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            &[
                (
                    "tests/test262-default-parameters-universe.txt",
                    TEST262_DEFAULT_PARAMETERS_MANIFEST_SHA256,
                ),
                (
                    "tests/test262-default-parameters-strict-body.txt",
                    TEST262_DEFAULT_PARAMETERS_STRICT_BODY_MANIFEST_SHA256,
                ),
            ],
        ),
        OxideProfileKind::Map => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_MAP_PROFILE_SHA256,
                "scoped Map Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped Map Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped Map Test262 capability profile requires its pinned manifest".to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped Map manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-map.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped Map manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped Map Test262 capability profile requires tests/test262-map.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_MAP_MANIFEST_SHA256,
                "scoped Map Test262 manifest",
            )?;
            Ok(TEST262_MAP_PROFILE_SHA256)
        }
        OxideProfileKind::Set => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_SET_PROFILE_SHA256,
                "scoped Set Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped Set Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped Set Test262 capability profile requires its pinned manifest".to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped Set manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-set.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped Set manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped Set Test262 capability profile requires tests/test262-set.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_SET_MANIFEST_SHA256,
                "scoped Set Test262 manifest",
            )?;
            Ok(TEST262_SET_PROFILE_SHA256)
        }
        OxideProfileKind::WeakCollections => verify_scoped_pinned_profile(
            options,
            "WeakMap/WeakSet",
            TEST262_WEAK_COLLECTIONS_PROFILE_SHA256,
            "tests/test262-weak-collections.txt",
            TEST262_WEAK_COLLECTIONS_MANIFEST_SHA256,
        ),
        OxideProfileKind::WeakCollectionsGlobalParent => verify_tag_transition_profile(
            options,
            "weak collections global admission",
            "parent",
            TEST262_WEAK_COLLECTIONS_GLOBAL_PARENT_PROFILE_SHA256,
            &[(
                "tests/test262-weak-collections-global-universe.txt",
                TEST262_WEAK_COLLECTIONS_GLOBAL_MANIFEST_SHA256,
            )],
        ),
        OxideProfileKind::WeakCollectionsGlobalCandidate => verify_tag_transition_profile(
            options,
            "weak collections global admission",
            "candidate",
            TEST262_WEAK_COLLECTIONS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            &[(
                "tests/test262-weak-collections-global-universe.txt",
                TEST262_WEAK_COLLECTIONS_GLOBAL_MANIFEST_SHA256,
            )],
        ),
        OxideProfileKind::SymbolProtocols => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256,
                "scoped well-known Symbol protocol Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped well-known Symbol protocol Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped well-known Symbol protocol Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped well-known Symbol protocol manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-symbol-protocols.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped well-known Symbol protocol manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped well-known Symbol protocol Test262 capability profile requires tests/test262-symbol-protocols.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_SYMBOL_PROTOCOLS_MANIFEST_SHA256,
                "scoped well-known Symbol protocol Test262 manifest",
            )?;
            Ok(TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256)
        }
        OxideProfileKind::RegExpBuiltins => verify_scoped_pinned_profile(
            options,
            "RegExp built-ins",
            TEST262_REGEXP_BUILTINS_PROFILE_SHA256,
            "tests/test262-regexp-builtins.txt",
            TEST262_REGEXP_BUILTINS_MANIFEST_SHA256,
        ),
        OxideProfileKind::GeneratorDestructuring => verify_scoped_pinned_profile(
            options,
            "synchronous generators and destructuring binding",
            TEST262_GENERATOR_DESTRUCTURING_PROFILE_SHA256,
            "tests/test262-generator-destructuring.txt",
            TEST262_GENERATOR_DESTRUCTURING_MANIFEST_SHA256,
        ),
        OxideProfileKind::IteratorHelpers => verify_scoped_pinned_profile(
            options,
            "synchronous Iterator helpers",
            TEST262_ITERATOR_HELPERS_PROFILE_SHA256,
            "tests/test262-iterator-helpers.txt",
            TEST262_ITERATOR_HELPERS_MANIFEST_SHA256,
        ),
        OxideProfileKind::IteratorHelpersGlobalParent => {
            verify_historical_global_transition_profile(
                options,
                "pre-R3bn Iterator helpers global parent",
                TEST262_ITERATOR_HELPERS_GLOBAL_PARENT_PROFILE_SHA256,
            )
        }
        OxideProfileKind::IteratorHelpersGlobalCandidate => {
            verify_historical_global_transition_profile(
                options,
                "R3bn Iterator helpers global candidate",
                TEST262_ITERATOR_HELPERS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            )
        }
        OxideProfileKind::GlobalThisParent => verify_scoped_pinned_profile(
            options,
            "pre-R3bo globalThis parent",
            TEST262_GLOBAL_THIS_PARENT_PROFILE_SHA256,
            "tests/test262-global-this-activation.txt",
            TEST262_GLOBAL_THIS_ACTIVATION_MANIFEST_SHA256,
        ),
        OxideProfileKind::GlobalThisCandidate => verify_scoped_pinned_profile(
            options,
            "globalThis activation candidate",
            TEST262_GLOBAL_THIS_CANDIDATE_PROFILE_SHA256,
            "tests/test262-global-this-activation.txt",
            TEST262_GLOBAL_THIS_ACTIVATION_MANIFEST_SHA256,
        ),
        OxideProfileKind::GlobalThisGlobalParent => verify_global_this_global_transition_profile(
            options,
            "pre-R3bp global parent",
            TEST262_GLOBAL_THIS_GLOBAL_PARENT_PROFILE_SHA256,
        ),
        OxideProfileKind::GlobalThisGlobalCandidate => {
            verify_global_this_global_transition_profile(
                options,
                "R3bp global candidate",
                TEST262_GLOBAL_THIS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            )
        }
        OxideProfileKind::PromiseGlobalParent => verify_promise_global_transition_profile(
            options,
            "pre-R3bq global parent",
            TEST262_PROMISE_GLOBAL_PARENT_PROFILE_SHA256,
        ),
        OxideProfileKind::PromiseGlobalCandidate => verify_promise_global_transition_profile(
            options,
            "R3bq global candidate",
            TEST262_PROMISE_GLOBAL_CANDIDATE_PROFILE_SHA256,
        ),
        OxideProfileKind::IteratorSequencing => verify_scoped_pinned_profile(
            options,
            "Iterator.concat sequencing",
            TEST262_ITERATOR_SEQUENCING_PROFILE_SHA256,
            "tests/test262-iterator-sequencing.txt",
            TEST262_ITERATOR_SEQUENCING_MANIFEST_SHA256,
        ),
        OxideProfileKind::OptionalChaining => verify_scoped_pinned_profile(
            options,
            "optional chaining",
            TEST262_OPTIONAL_CHAINING_PROFILE_SHA256,
            "tests/test262-optional-chaining.txt",
            TEST262_OPTIONAL_CHAINING_MANIFEST_SHA256,
        ),
        OxideProfileKind::Proxy => verify_scoped_pinned_profile(
            options,
            "Proxy",
            TEST262_PROXY_PROFILE_SHA256,
            "tests/test262-proxy.txt",
            TEST262_PROXY_MANIFEST_SHA256,
        ),
    }
}

fn validate_oxide_profile(profile: &OxideProfile, suite: &Path) -> Result<(), String> {
    for relative in profile.audited_negative_paths() {
        let relative = Path::new(relative);
        validate_relative_test_path(relative)?;
        let path = suite.join(relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read audited negative {}: {error}", path.display()))?;
        let metadata = parse_metadata(&source).map_err(|error| {
            format!(
                "parse metadata for audited negative {}: {error}",
                relative.display()
            )
        })?;
        if metadata.negative.is_none() {
            return Err(format!(
                "oxide profile path is not a negative test: {}",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn missing_host_result(missing: &[String]) -> Option<WorkerResult> {
    if missing.is_empty() {
        return None;
    }
    let has_module = missing.iter().any(|capability| capability == "module");
    let has_async = missing.iter().any(|capability| capability == "async");
    let detail = format!("missing execution capabilities: {}", missing.join(", "));
    if has_module || has_async {
        let outcome = match (has_module, has_async) {
            (true, true) => "unsupported-module-async",
            (true, false) => "unsupported-module",
            (false, true) => "unsupported-async",
            (false, false) => unreachable!(),
        };
        return Some(WorkerResult::failure(
            outcome,
            "selection",
            "ExecutionMode",
            detail,
        ));
    }

    let first = missing.first().expect("missing capabilities were checked");
    let outcome = match first.as_str() {
        "abstract-module-source" => "unsupported-host-abstract-module-source",
        "agent" => "unsupported-host-agent",
        "can-block:false" => "unsupported-host-can-block-false",
        "create-realm" => "unsupported-host-create-realm",
        "detach-array-buffer" => "unsupported-host-detach-array-buffer",
        "eval-script" => "unsupported-host-eval-script",
        "gc" => "unsupported-host-gc",
        "global" => "unsupported-host-global",
        "is-html-dda" => "unsupported-host-is-html-dda",
        unknown if unknown.starts_with("unknown:") => "unsupported-host-unknown-hook",
        _ => "unsupported-host",
    };
    Some(WorkerResult::failure(
        outcome,
        "selection",
        "HostCapability",
        detail,
    ))
}

fn audit_metadata(options: &MetadataAuditOptions) -> Result<(), String> {
    validate_suite(&options.suite)?;
    let mut tests = Vec::new();
    collect_js_files(&options.suite.join("test"), &options.suite, &mut tests)?;
    sort_test_paths(&mut tests);
    if tests.len() != 53_125 {
        return Err(format!(
            "pinned metadata inventory has {} tests instead of 53125",
            tests.len()
        ));
    }

    let mut records = Vec::new();
    let mut counts = BTreeMap::<String, usize>::new();
    for relative in tests {
        let path = options.suite.join(&relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if !source.contains("/*---") {
            return Err(format!("frontmatter is missing: {}", relative.display()));
        }
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse metadata for {}: {error}", relative.display()))?;
        let (phase, error_type) = if let Some(negative) = &metadata.negative {
            let phase = negative
                .phase
                .as_deref()
                .ok_or_else(|| format!("negative phase is missing: {}", relative.display()))?;
            if !matches!(phase, "parse" | "resolution" | "runtime") {
                return Err(format!(
                    "unknown negative phase {phase:?}: {}",
                    relative.display()
                ));
            }
            let error_type = negative
                .error_type
                .as_deref()
                .ok_or_else(|| format!("negative type is missing: {}", relative.display()))?;
            *counts.entry("negative".to_owned()).or_default() += 1;
            *counts.entry(format!("phase:{phase}")).or_default() += 1;
            (phase, error_type)
        } else {
            *counts.entry("positive".to_owned()).or_default() += 1;
            ("", "")
        };
        for flag in ["raw", "module", "async", "noStrict", "onlyStrict"] {
            if metadata.flags.contains(flag) {
                *counts.entry(flag.to_owned()).or_default() += 1;
            }
        }

        write_record_field(&mut records, &relative.to_string_lossy());
        write_record_field(&mut records, &metadata.includes.join(","));
        write_record_field(
            &mut records,
            &metadata.flags.iter().cloned().collect::<Vec<_>>().join(","),
        );
        write_record_field(&mut records, &metadata.features.join(","));
        write_record_field(&mut records, phase);
        records.extend_from_slice(error_type.as_bytes());
        records.push(b'\n');
    }

    if let Some(parent) = options
        .records
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(&options.records, records)
        .map_err(|error| format!("write {}: {error}", options.records.display()))?;
    println!("Test262 metadata: files=53125");
    for name in [
        "raw",
        "module",
        "async",
        "noStrict",
        "onlyStrict",
        "positive",
        "negative",
        "phase:parse",
        "phase:resolution",
        "phase:runtime",
    ] {
        println!("{name}={}", counts.get(name).copied().unwrap_or(0));
    }
    println!("records={}", options.records.display());
    Ok(())
}

fn write_record_field(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(value.as_bytes());
    output.push(0);
}

fn collect_tests(options: &CoordinatorOptions) -> Result<Vec<PathBuf>, String> {
    let values = if let Some(manifest) = &options.manifest {
        fs::read_to_string(manifest)
            .map_err(|error| format!("read {}: {error}", manifest.display()))?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    } else if options.all {
        let mut values = Vec::new();
        collect_js_files(&options.suite.join("test"), &options.suite, &mut values)?;
        values
    } else {
        options.tests.clone()
    };
    let mut unique = BTreeSet::new();
    for value in values {
        validate_relative_test_path(&value)?;
        if !options.suite.join(&value).is_file() {
            return Err(format!("test file is missing: {}", value.display()));
        }
        if !unique.insert(value.clone()) {
            return Err(format!("duplicate test path: {}", value.display()));
        }
    }
    if unique.is_empty() {
        return Err("test selection is empty".to_owned());
    }
    let mut tests = unique.into_iter().collect::<Vec<_>>();
    sort_test_paths(&mut tests);
    Ok(tests)
}

fn sort_test_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|left, right| {
        left.as_os_str()
            .as_encoded_bytes()
            .cmp(right.as_os_str().as_encoded_bytes())
    });
}

fn collect_js_files(
    directory: &Path,
    suite: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_js_files(&path, suite, output)?;
        } else if file_type.is_file()
            && path.extension().is_some_and(|extension| extension == "js")
            && !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("_FIXTURE.js"))
        {
            output.push(
                path.strip_prefix(suite)
                    .map_err(|_| format!("{} escaped suite root", path.display()))?
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn validate_relative_test_path(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !path.starts_with("test")
        || path.extension().is_none_or(|extension| extension != "js")
        || path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains("_FIXTURE"))
    {
        return Err(format!("invalid relative Test262 path: {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{
        Invocation, OxideProfileKind, TEST262_AGGREGATE_ERROR_PROFILE_SHA256,
        TEST262_ARGUMENT_SPREAD_PROFILE_SHA256, TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256,
        TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256, TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256,
        TEST262_ARRAY_BUFFER_PROFILE_SHA256, TEST262_ASYNC_ARROW_CORE_PROFILE_SHA256,
        TEST262_ASYNC_CLASS_METHOD_CORE_PROFILE_SHA256, TEST262_ASYNC_FUNCTION_CORE_PROFILE_SHA256,
        TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_PROFILE_SHA256,
        TEST262_ASYNC_GENERATOR_CORE_PROFILE_SHA256,
        TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_PROFILE_SHA256,
        TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256,
        TEST262_ASYNC_GENERATOR_YIELD_STAR_PROFILE_SHA256,
        TEST262_ASYNC_OBJECT_METHOD_CORE_PROFILE_SHA256,
        TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256,
        TEST262_CATCH_BINDING_PROFILE_SHA256, TEST262_CLASS_BASE_PROFILE_SHA256,
        TEST262_CLASS_DERIVED_PROFILE_SHA256, TEST262_CLASS_GENERATOR_METHODS_PROFILE_SHA256,
        TEST262_CLASS_PRIVATE_ACCESSORS_PROFILE_SHA256,
        TEST262_CLASS_PRIVATE_FIELDS_PROFILE_SHA256,
        TEST262_CLASS_PRIVATE_GENERATOR_METHODS_PROFILE_SHA256,
        TEST262_CLASS_PRIVATE_METHODS_PROFILE_SHA256, TEST262_CLASS_PUBLIC_INIT_PROFILE_SHA256,
        TEST262_CLASS_SYNC_MATRIX_PROFILE_SHA256,
        TEST262_COMPUTED_PROPERTY_NAMES_CANDIDATE_PROFILE_SHA256,
        TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_COMPUTED_PROPERTY_NAMES_PARENT_PROFILE_SHA256,
        TEST262_DATA_VIEW_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_DATA_VIEW_GLOBAL_PARENT_PROFILE_SHA256, TEST262_DATA_VIEW_PROFILE_SHA256,
        TEST262_DEFAULT_PARAMETERS_CANDIDATE_PROFILE_SHA256,
        TEST262_DEFAULT_PARAMETERS_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_DEFAULT_PARAMETERS_PARENT_PROFILE_SHA256,
        TEST262_GENERATOR_DESTRUCTURING_PROFILE_SHA256,
        TEST262_GLOBAL_THIS_CANDIDATE_PROFILE_SHA256,
        TEST262_GLOBAL_THIS_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_GLOBAL_THIS_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_GLOBAL_THIS_PARENT_PROFILE_SHA256, TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256,
        TEST262_IDENTIFIER_REST_PROFILE_SHA256,
        TEST262_ITERATOR_HELPERS_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_ITERATOR_HELPERS_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_ITERATOR_HELPERS_PROFILE_SHA256, TEST262_ITERATOR_SEQUENCING_PROFILE_SHA256,
        TEST262_MAP_PROFILE_SHA256, TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256,
        TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256,
        TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256, TEST262_OBJECT_BINDING_PROFILE_SHA256,
        TEST262_OBJECT_REST_BINDING_PROFILE_SHA256,
        TEST262_OBJECT_REST_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_OBJECT_REST_GLOBAL_PARENT_PROFILE_SHA256, TEST262_OPTIONAL_CHAINING_PROFILE_SHA256,
        TEST262_PARAMETER_BINDING_PATTERNS_PROFILE_SHA256,
        TEST262_PARAMETER_DIRECT_EVAL_PROFILE_SHA256,
        TEST262_PARAMETER_EXPRESSION_BINDING_PATTERNS_PROFILE_SHA256,
        TEST262_PROMISE_ALL_PROFILE_SHA256, TEST262_PROMISE_ALL_SETTLED_PROFILE_SHA256,
        TEST262_PROMISE_ANY_PROFILE_SHA256, TEST262_PROMISE_FINALLY_PROFILE_SHA256,
        TEST262_PROMISE_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_PROMISE_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_PROMISE_RACE_TRY_WITH_RESOLVERS_PROFILE_SHA256, TEST262_PROXY_PROFILE_SHA256,
        TEST262_REGEXP_BUILTINS_PROFILE_SHA256,
        TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_RESIZABLE_ARRAYBUFFER_PROFILE_SHA256,
        TEST262_REST_PARAMETERS_CANDIDATE_PROFILE_SHA256,
        TEST262_REST_PARAMETERS_PARENT_PROFILE_SHA256, TEST262_SET_PROFILE_SHA256,
        TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256, TEST262_TYPED_ARRAY_CORE_PROFILE_SHA256,
        TEST262_UINT8ARRAY_CODECS_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_UINT8ARRAY_CODECS_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_UINT8ARRAY_CODECS_PROFILE_SHA256,
        TEST262_WEAK_COLLECTIONS_GLOBAL_CANDIDATE_PROFILE_SHA256,
        TEST262_WEAK_COLLECTIONS_GLOBAL_PARENT_PROFILE_SHA256,
        TEST262_WEAK_COLLECTIONS_PROFILE_SHA256, default_worker_count, identify_oxide_profile,
        parse_args, verify_oxide_profile, verify_scoped_pinned_profile,
    };

    fn parse(values: &[&str]) -> Result<Invocation, String> {
        parse_args(values.iter().map(OsString::from))
    }

    fn parse_error(values: &[&str]) -> String {
        match parse(values) {
            Ok(_) => panic!("arguments unexpectedly parsed"),
            Err(error) => error,
        }
    }

    #[test]
    fn coordinator_accepts_an_explicit_positive_worker_bound() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "compat/test262-oxide.conf",
            "--manifest",
            "manifest",
            "--report",
            "report.tsv",
            "--workers",
            "3",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(options.workers, 3);
    }

    #[test]
    fn zero_workers_and_missing_profile_are_rejected() {
        let zero = parse_error(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "profile",
            "--all",
            "--report",
            "report.tsv",
            "--workers",
            "0",
        ]);
        assert_eq!(zero, "--workers must be greater than zero");

        let missing = parse_error(&["--suite", "suite", "--all", "--report", "report.tsv"]);
        assert_eq!(missing, "--oxide-profile is required");
    }

    #[test]
    fn internal_and_metadata_modes_reject_coordinator_tuning() {
        let audit = parse_error(&[
            "--suite",
            "suite",
            "--validate-metadata",
            "records",
            "--workers",
            "2",
        ]);
        assert!(audit.contains("cannot be combined"));

        let worker = parse_error(&[
            "--worker-one",
            "--suite",
            "suite",
            "--test",
            "test/a.js",
            "--variant",
            "sloppy",
            "--timeout-ms",
            "10",
        ]);
        assert_eq!(worker, "invalid coordinator option passed to --worker-one");

        let coordinator = parse_error(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "profile",
            "--all",
            "--report",
            "report.tsv",
            "--allow-async-host",
        ]);
        assert_eq!(
            coordinator,
            "--allow-async-host is internal to --worker-one"
        );
    }

    #[test]
    fn worker_accepts_the_explicit_scoped_async_host_flag() {
        let invocation = parse(&[
            "--worker-one",
            "--suite",
            "suite",
            "--test",
            "test/a.js",
            "--variant",
            "sloppy",
            "--allow-async-host",
        ])
        .unwrap();
        let Invocation::Worker(options) = invocation else {
            panic!("worker arguments selected another invocation");
        };
        assert!(options.allow_async_host);
    }

    #[test]
    fn automatic_worker_bound_is_nonzero_and_capped() {
        assert!((1..=16).contains(&default_worker_count()));
    }

    #[test]
    fn only_pinned_global_and_scoped_profiles_are_accepted() {
        assert_eq!(
            identify_oxide_profile(Path::new("compat/test262-oxide.conf")).unwrap(),
            OxideProfileKind::Global
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-aggregate-error.conf")).unwrap(),
            OxideProfileKind::AggregateError
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-argument-spread.conf")).unwrap(),
            OxideProfileKind::ArgumentSpread
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-async-function-core.conf")).unwrap(),
            OxideProfileKind::AsyncFunctionCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-async-generator-core.conf")).unwrap(),
            OxideProfileKind::AsyncGeneratorCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-async-generator-object-method-core.conf"
            ))
            .unwrap(),
            OxideProfileKind::AsyncGeneratorObjectMethodCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-async-generator-class-method-core.conf"
            ))
            .unwrap(),
            OxideProfileKind::AsyncGeneratorClassMethodCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-async-generator-private-class-method-core.conf"
            ))
            .unwrap(),
            OxideProfileKind::AsyncGeneratorPrivateClassMethodCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-async-generator-yield-star.conf"))
                .unwrap(),
            OxideProfileKind::AsyncGeneratorYieldStar
        );
        assert_eq!(
            identify_oxide_profile(Path::new("compat/test262-for-await-of.conf")).unwrap(),
            OxideProfileKind::ForAwaitOf
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-async-arrow-core.conf")).unwrap(),
            OxideProfileKind::AsyncArrowCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-async-object-method-core.conf"))
                .unwrap(),
            OxideProfileKind::AsyncObjectMethodCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-async-class-method-core.conf"))
                .unwrap(),
            OxideProfileKind::AsyncClassMethodCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-async-private-class-method-core.conf",
            ))
            .unwrap(),
            OxideProfileKind::AsyncPrivateClassMethodCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-base.conf")).unwrap(),
            OxideProfileKind::ClassBase
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-derived.conf")).unwrap(),
            OxideProfileKind::ClassDerived
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-sync-matrix.conf")).unwrap(),
            OxideProfileKind::ClassSyncMatrix
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-public-init.conf")).unwrap(),
            OxideProfileKind::ClassPublicInit
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-private-fields.conf")).unwrap(),
            OxideProfileKind::ClassPrivateFields
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-private-methods.conf")).unwrap(),
            OxideProfileKind::ClassPrivateMethods
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-private-accessors.conf"))
                .unwrap(),
            OxideProfileKind::ClassPrivateAccessors
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-class-generator-methods.conf"))
                .unwrap(),
            OxideProfileKind::ClassGeneratorMethods
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-class-private-generator-methods.conf",
            ))
            .unwrap(),
            OxideProfileKind::ClassPrivateGeneratorMethods
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-constructor-jobs.conf",))
                .unwrap(),
            OxideProfileKind::PromiseConstructorJobs
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-promise-race-try-with-resolvers.conf",
            ))
            .unwrap(),
            OxideProfileKind::PromiseRaceTryWithResolvers
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-finally.conf")).unwrap(),
            OxideProfileKind::PromiseFinally
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-all.conf")).unwrap(),
            OxideProfileKind::PromiseAll
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-all-settled.conf")).unwrap(),
            OxideProfileKind::PromiseAllSettled
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-any.conf")).unwrap(),
            OxideProfileKind::PromiseAny
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-binding-flat.conf")).unwrap(),
            OxideProfileKind::ArrayBindingFlat
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-binding-nested.conf")).unwrap(),
            OxideProfileKind::ArrayBindingNested
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-assignment-flat.conf")).unwrap(),
            OxideProfileKind::ArrayAssignmentFlat
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-catch-binding.conf")).unwrap(),
            OxideProfileKind::CatchBinding
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-identifier-defaults.conf")).unwrap(),
            OxideProfileKind::IdentifierDefaults
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-parameter-direct-eval.conf")).unwrap(),
            OxideProfileKind::ParameterDirectEval
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-parameter-binding-patterns.conf"))
                .unwrap(),
            OxideProfileKind::ParameterBindingPatterns
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-parameter-expression-binding-patterns.conf"
            ))
            .unwrap(),
            OxideProfileKind::ParameterExpressionBindingPatterns
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-identifier-rest.conf")).unwrap(),
            OxideProfileKind::IdentifierRest
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-assignment-flat.conf")).unwrap(),
            OxideProfileKind::ObjectAssignmentFlat
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-assignment-nested.conf"))
                .unwrap(),
            OxideProfileKind::ObjectAssignmentNested
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-assignment-rest.conf")).unwrap(),
            OxideProfileKind::ObjectAssignmentRest
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-binding.conf")).unwrap(),
            OxideProfileKind::ObjectBinding
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-rest-binding.conf")).unwrap(),
            OxideProfileKind::ObjectRestBinding
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-rest-global-parent.conf"))
                .unwrap(),
            OxideProfileKind::ObjectRestGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-rest-global-candidate.conf"))
                .unwrap(),
            OxideProfileKind::ObjectRestGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-buffer.conf")).unwrap(),
            OxideProfileKind::ArrayBuffer
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-data-view.conf")).unwrap(),
            OxideProfileKind::DataView
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-data-view-global-parent.conf"))
                .unwrap(),
            OxideProfileKind::DataViewGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-data-view-global-candidate.conf"))
                .unwrap(),
            OxideProfileKind::DataViewGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-typed-array-core.conf")).unwrap(),
            OxideProfileKind::TypedArrayCore
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-uint8array-codecs.conf")).unwrap(),
            OxideProfileKind::Uint8ArrayCodecs
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-uint8array-codecs-global-parent.conf"
            ))
            .unwrap(),
            OxideProfileKind::Uint8ArrayCodecsGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-uint8array-codecs-global-candidate.conf"
            ))
            .unwrap(),
            OxideProfileKind::Uint8ArrayCodecsGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-resizable-arraybuffer.conf")).unwrap(),
            OxideProfileKind::ResizableArrayBuffer
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-resizable-arraybuffer-global-parent.conf"
            ))
            .unwrap(),
            OxideProfileKind::ResizableArrayBufferGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-resizable-arraybuffer-global-candidate.conf"
            ))
            .unwrap(),
            OxideProfileKind::ResizableArrayBufferGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-computed-property-names.conf"))
                .unwrap(),
            OxideProfileKind::ComputedPropertyNamesCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-computed-property-names-parent.conf"
            ))
            .unwrap(),
            OxideProfileKind::ComputedPropertyNamesParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-computed-property-names-global-parent.conf"
            ))
            .unwrap(),
            OxideProfileKind::ComputedPropertyNamesGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-computed-property-names-global-candidate.conf"
            ))
            .unwrap(),
            OxideProfileKind::ComputedPropertyNamesGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-rest-parameters-parent.conf")).unwrap(),
            OxideProfileKind::RestParametersParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-rest-parameters-candidate.conf"))
                .unwrap(),
            OxideProfileKind::RestParametersCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-default-parameters-parent.conf"))
                .unwrap(),
            OxideProfileKind::DefaultParametersParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-default-parameters-candidate.conf"))
                .unwrap(),
            OxideProfileKind::DefaultParametersCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-default-parameters-global-candidate.conf"
            ))
            .unwrap(),
            OxideProfileKind::DefaultParametersGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-map.conf")).unwrap(),
            OxideProfileKind::Map
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-set.conf")).unwrap(),
            OxideProfileKind::Set
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-weak-collections.conf")).unwrap(),
            OxideProfileKind::WeakCollections
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-weak-collections-global-parent.conf"
            ))
            .unwrap(),
            OxideProfileKind::WeakCollectionsGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-weak-collections-global-candidate.conf"
            ))
            .unwrap(),
            OxideProfileKind::WeakCollectionsGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-symbol-protocols.conf")).unwrap(),
            OxideProfileKind::SymbolProtocols
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-generator-destructuring.conf"))
                .unwrap(),
            OxideProfileKind::GeneratorDestructuring
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-iterator-helpers.conf")).unwrap(),
            OxideProfileKind::IteratorHelpers
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-iterator-helpers-global-parent.conf"
            ))
            .unwrap(),
            OxideProfileKind::IteratorHelpersGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new(
                "tests/test262-iterator-helpers-global-candidate.conf"
            ))
            .unwrap(),
            OxideProfileKind::IteratorHelpersGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-global-this-parent.conf")).unwrap(),
            OxideProfileKind::GlobalThisParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-global-this-candidate.conf")).unwrap(),
            OxideProfileKind::GlobalThisCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-global-this-global-parent.conf"))
                .unwrap(),
            OxideProfileKind::GlobalThisGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-global-this-global-candidate.conf"))
                .unwrap(),
            OxideProfileKind::GlobalThisGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-global-parent.conf")).unwrap(),
            OxideProfileKind::PromiseGlobalParent
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-promise-global-candidate.conf"))
                .unwrap(),
            OxideProfileKind::PromiseGlobalCandidate
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-iterator-sequencing.conf")).unwrap(),
            OxideProfileKind::IteratorSequencing
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-optional-chaining.conf")).unwrap(),
            OxideProfileKind::OptionalChaining
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-proxy.conf")).unwrap(),
            OxideProfileKind::Proxy
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-regexp-builtins.conf")).unwrap(),
            OxideProfileKind::RegExpBuiltins
        );

        let error = identify_oxide_profile(Path::new("Cargo.toml")).unwrap_err();
        assert!(error.contains("unsupported Test262 capability profile"));
    }

    #[test]
    fn scoped_aggregate_error_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-aggregate-error.conf",
            "--manifest",
            "tests/test262-aggregate-error.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_AGGREGATE_ERROR_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/AggregateError/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-aggregate-error.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_argument_spread_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-argument-spread.conf",
            "--manifest",
            "tests/test262-argument-spread.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARGUMENT_SPREAD_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/language/expressions/call/spread-obj.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-argument-spread.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_function_core_profile_is_bound_and_detects_manifest_tampering() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-function-core.conf",
            "--manifest",
            "tests/test262-async-function-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_FUNCTION_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "ordinary async function core",
            TEST262_ASYNC_FUNCTION_CORE_PROFILE_SHA256,
            "tests/test262-async-function-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/async-function/evaluation.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-function-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_generator_core_profile_is_bound_to_its_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-generator-core.conf",
            "--manifest",
            "tests/test262-async-generator-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_GENERATOR_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "ordinary async-generator function core",
            TEST262_ASYNC_GENERATOR_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
    }

    #[test]
    fn scoped_async_generator_object_method_profile_is_bound_to_its_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-generator-object-method-core.conf",
            "--manifest",
            "tests/test262-async-generator-object-method-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "async-generator object method core",
            TEST262_ASYNC_GENERATOR_OBJECT_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-object-method-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/built-ins/Function/prototype/toString/async-generator-method-object.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-generator-object-method-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_generator_class_method_profile_is_bound_to_its_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-generator-class-method-core.conf",
            "--manifest",
            "tests/test262-async-generator-class-method-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "public async-generator class method core",
            TEST262_ASYNC_GENERATOR_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-class-method-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/built-ins/Function/prototype/toString/async-generator-method-class-expression.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-generator-class-method-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_generator_private_class_method_profile_is_bound_to_its_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-generator-private-class-method-core.conf",
            "--manifest",
            "tests/test262-async-generator-private-class-method-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "private async-generator class method core",
            TEST262_ASYNC_GENERATOR_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-generator-private-class-method-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/class/elements/async-gen-private-method-static/await-as-binding-identifier.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-generator-private-class-method-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_generator_yield_star_profile_is_bound_to_its_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-generator-yield-star.conf",
            "--manifest",
            "tests/test262-async-generator-yield-star.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_GENERATOR_YIELD_STAR_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "async-generator yield-star",
            TEST262_ASYNC_GENERATOR_YIELD_STAR_PROFILE_SHA256,
            "tests/test262-async-generator-yield-star.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/async-generator/named-yield-star-async-next.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-generator-yield-star.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_arrow_core_profile_is_bound_and_detects_manifest_tampering() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-arrow-core.conf",
            "--manifest",
            "tests/test262-async-arrow-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_ARROW_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "async arrow core",
            TEST262_ASYNC_ARROW_CORE_PROFILE_SHA256,
            "tests/test262-async-arrow-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/async-arrow-function/arrow-returns-promise.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-arrow-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_object_method_core_profile_is_bound_and_detects_manifest_tampering() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-object-method-core.conf",
            "--manifest",
            "tests/test262-async-object-method-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_OBJECT_METHOD_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "ordinary async object method core",
            TEST262_ASYNC_OBJECT_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-object-method-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/object/method-definition/object-method-returns-promise.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-object-method-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_class_method_core_profile_is_bound_and_detects_manifest_tampering() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-class-method-core.conf",
            "--manifest",
            "tests/test262-async-class-method-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_CLASS_METHOD_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "public ordinary async class method core",
            TEST262_ASYNC_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-class-method-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/class/definition/class-method-returns-promise.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-class-method-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_async_private_class_method_core_profile_is_bound_and_detects_manifest_tampering() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-async-private-class-method-core.conf",
            "--manifest",
            "tests/test262-async-private-class-method-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256
        );

        let tamper_error = verify_scoped_pinned_profile(
            &options,
            "ordinary private async class method core",
            TEST262_ASYNC_PRIVATE_CLASS_METHOD_CORE_PROFILE_SHA256,
            "tests/test262-async-private-class-method-core.txt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(
            tamper_error.contains("manifest checksum mismatch"),
            "unexpected manifest tamper error: {tamper_error}"
        );
        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/class/elements/private-methods/prod-private-async-method.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-async-private-class-method-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_base_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-base.conf",
            "--manifest",
            "tests/test262-class-base.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_BASE_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/built-ins/Function/internals/Call/class-ctor.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-base.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_derived_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-derived.conf",
            "--manifest",
            "tests/test262-class-derived.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_DERIVED_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/class/subclass/default-constructor.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-derived.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_sync_matrix_profile_is_bound_to_its_pinned_manifest_and_fails_closed() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-sync-matrix.conf",
            "--manifest",
            "tests/test262-class-sync-matrix.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_SYNC_MATRIX_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/class/subclass/default-constructor.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-sync-matrix.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            let error = verify_oxide_profile(&options).unwrap_err();
            assert!(
                error.contains("requires its pinned manifest")
                    || error.contains("requires tests/test262-class-sync-matrix.txt"),
                "unexpected fail-closed error: {error}"
            );
        }
    }

    #[test]
    fn scoped_class_public_init_profile_is_bound_and_admits_only_its_reviewed_features() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-public-init.conf",
            "--manifest",
            "tests/test262-class-public-init.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_PUBLIC_INIT_PROFILE_SHA256
        );

        let profile =
            super::OxideProfile::load(Path::new("tests/test262-class-public-init.conf")).unwrap();
        let positive = Path::new(
            "test/language/expressions/class/constructor-this-tdz-during-initializers.js",
        );
        for feature in [
            "class",
            "class-fields-public",
            "class-static-fields-public",
            "class-static-block",
        ] {
            assert_eq!(
                profile.classify(positive, &[feature.to_owned()], false),
                None,
                "scoped profile did not admit {feature}"
            );
        }
        let audited_negative =
            Path::new("test/language/expressions/class/elements/fields-asi-3.js");
        assert_eq!(
            profile.classify(
                audited_negative,
                &["class".to_owned(), "class-fields-public".to_owned()],
                true,
            ),
            None
        );
        let adjacent = profile
            .classify(
                positive,
                &[
                    "class-fields-public".to_owned(),
                    "computed-property-names".to_owned(),
                ],
                false,
            )
            .unwrap();
        assert_eq!(adjacent.outcome, "unsupported-feature");
        assert!(adjacent.detail.ends_with("computed-property-names"));

        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        for feature in [
            "class",
            "class-fields-public",
            "class-static-fields-public",
            "class-static-block",
        ] {
            assert_eq!(
                global
                    .classify(positive, &[feature.to_owned()], false)
                    .unwrap()
                    .outcome,
                "unsupported-feature",
                "global profile unexpectedly admitted {feature}"
            );
        }

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/class/constructor-this-tdz-during-initializers.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-public-init.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_private_fields_profile_is_bound_and_admits_only_its_reviewed_features() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-private-fields.conf",
            "--manifest",
            "tests/test262-class-private-fields.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_PRIVATE_FIELDS_PROFILE_SHA256
        );

        let profile =
            super::OxideProfile::load(Path::new("tests/test262-class-private-fields.conf"))
                .unwrap();
        let positive = Path::new(
            "test/language/expressions/class/elements/regular-definitions-private-field-usage.js",
        );
        for feature in [
            "arrow-function",
            "class",
            "class-fields-private",
            "class-fields-private-in",
            "class-fields-public",
            "class-static-fields-private",
            "class-static-fields-public",
        ] {
            assert_eq!(
                profile.classify(positive, &[feature.to_owned()], false),
                None,
                "scoped profile did not admit {feature}"
            );
        }
        let audited_negative =
            Path::new("test/language/expressions/class/elements/fields-duplicate-privatenames.js");
        assert_eq!(
            profile.classify(
                audited_negative,
                &["class".to_owned(), "class-fields-private".to_owned()],
                true,
            ),
            None
        );
        let adjacent = profile
            .classify(
                positive,
                &[
                    "class-fields-private".to_owned(),
                    "class-methods-private".to_owned(),
                ],
                false,
            )
            .unwrap();
        assert_eq!(adjacent.outcome, "unsupported-feature");
        assert!(adjacent.detail.ends_with("class-methods-private"));

        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        for feature in [
            "class-fields-private",
            "class-fields-private-in",
            "class-static-fields-private",
        ] {
            assert_eq!(
                global
                    .classify(positive, &[feature.to_owned()], false)
                    .unwrap()
                    .outcome,
                "unsupported-feature",
                "global profile unexpectedly admitted {feature}"
            );
        }

        for selection in [
            ["--all", ""],
            ["--test", positive.to_str().unwrap()],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-private-fields.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_private_methods_profile_is_bound_and_excludes_accessors_and_adjacencies() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-private-methods.conf",
            "--manifest",
            "tests/test262-class-private-methods.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_PRIVATE_METHODS_PROFILE_SHA256
        );

        let profile =
            super::OxideProfile::load(Path::new("tests/test262-class-private-methods.conf"))
                .unwrap();
        let positive =
            Path::new("test/language/statements/class/elements/private-method-brand-check.js");
        for feature in [
            "class",
            "class-fields-private",
            "class-fields-private-in",
            "class-fields-public",
            "class-methods-private",
            "class-static-fields-private",
            "class-static-fields-public",
            "class-static-methods-private",
        ] {
            assert_eq!(
                profile.classify(positive, &[feature.to_owned()], false),
                None,
                "scoped profile did not admit {feature}"
            );
        }
        let audited_negative = Path::new(
            "test/language/statements/class/elements/syntax/early-errors/grammar-privatemeth-duplicate-meth-meth.js",
        );
        assert_eq!(
            profile.classify(
                audited_negative,
                &["class".to_owned(), "class-methods-private".to_owned()],
                true,
            ),
            None
        );
        let adjacent = profile
            .classify(
                positive,
                &["class-methods-private".to_owned(), "generators".to_owned()],
                false,
            )
            .unwrap();
        assert_eq!(adjacent.outcome, "unsupported-feature");
        assert!(adjacent.detail.ends_with("generators"));

        let unaudited_accessor =
            Path::new("test/language/statements/class/elements/private-getter-brand-check.js");
        assert_eq!(
            profile
                .classify(
                    unaudited_accessor,
                    &["class".to_owned(), "class-methods-private".to_owned()],
                    true,
                )
                .unwrap()
                .outcome,
            "unsupported-negative-provenance"
        );

        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        for feature in ["class-methods-private", "class-static-methods-private"] {
            assert_eq!(
                global
                    .classify(positive, &[feature.to_owned()], false)
                    .unwrap()
                    .outcome,
                "unsupported-feature",
                "global profile unexpectedly admitted {feature}"
            );
        }

        for selection in [
            ["--all", ""],
            ["--test", positive.to_str().unwrap()],
            ["--manifest", "tests/test262-class-private-fields.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-private-methods.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_private_accessors_profile_is_bound_to_its_audited_partition() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-private-accessors.conf",
            "--manifest",
            "tests/test262-class-private-accessors.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_PRIVATE_ACCESSORS_PROFILE_SHA256
        );

        let profile =
            super::OxideProfile::load(Path::new("tests/test262-class-private-accessors.conf"))
                .unwrap();
        let positive = Path::new(
            "test/language/statements/class/elements/private-getter-brand-check-multiple-evaluations-of-class.js",
        );
        for feature in [
            "class",
            "class-fields-private",
            "class-fields-private-in",
            "class-fields-public",
            "class-methods-private",
            "class-static-fields-private",
            "class-static-fields-public",
            "class-static-methods-private",
        ] {
            assert_eq!(
                profile.classify(positive, &[feature.to_owned()], false),
                None,
                "scoped profile did not admit {feature}"
            );
        }
        let audited_negative = Path::new(
            "test/language/statements/class/elements/syntax/early-errors/grammar-privatemeth-duplicate-get-get.js",
        );
        assert_eq!(
            profile.classify(
                audited_negative,
                &["class".to_owned(), "class-methods-private".to_owned()],
                true,
            ),
            None
        );

        let unaudited_method = Path::new(
            "test/language/statements/class/elements/syntax/early-errors/grammar-privatemeth-duplicate-meth-meth.js",
        );
        assert_eq!(
            profile
                .classify(
                    unaudited_method,
                    &["class".to_owned(), "class-methods-private".to_owned()],
                    true,
                )
                .unwrap()
                .outcome,
            "unsupported-negative-provenance"
        );
        let adjacent = profile
            .classify(
                positive,
                &["class-methods-private".to_owned(), "generators".to_owned()],
                false,
            )
            .unwrap();
        assert_eq!(adjacent.outcome, "unsupported-feature");
        assert!(adjacent.detail.ends_with("generators"));

        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        for feature in ["class-methods-private", "class-static-methods-private"] {
            assert_eq!(
                global
                    .classify(positive, &[feature.to_owned()], false)
                    .unwrap()
                    .outcome,
                "unsupported-feature",
                "global profile unexpectedly admitted {feature}"
            );
        }

        for selection in [
            ["--all", ""],
            ["--test", positive.to_str().unwrap()],
            ["--manifest", "tests/test262-class-private-methods.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-private-accessors.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_generator_methods_profile_is_bound_to_its_audited_partition() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-generator-methods.conf",
            "--manifest",
            "tests/test262-class-generator-methods.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_GENERATOR_METHODS_PROFILE_SHA256
        );

        let profile =
            super::OxideProfile::load(Path::new("tests/test262-class-generator-methods.conf"))
                .unwrap();
        let positive = Path::new(
            "test/language/statements/class/definition/methods-gen-yield-star-before-newline.js",
        );
        assert_eq!(
            profile.classify(positive, &["generators".to_owned()], false),
            None
        );
        let audited_negative = Path::new(
            "test/language/statements/class/definition/methods-gen-yield-star-after-newline.js",
        );
        assert_eq!(
            profile.classify(audited_negative, &["generators".to_owned()], true),
            None
        );
        assert_eq!(
            profile
                .classify(positive, &["generators".to_owned()], true)
                .unwrap()
                .outcome,
            "unsupported-negative-provenance"
        );

        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        assert_eq!(
            global.classify(positive, &["generators".to_owned()], false),
            None,
            "global profile should admit the globally audited generators feature"
        );

        for selection in [
            ["--all", ""],
            ["--test", positive.to_str().unwrap()],
            ["--manifest", "tests/test262-class-private-accessors.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-generator-methods.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_class_private_generator_methods_profile_is_bound_to_its_audited_partition() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-class-private-generator-methods.conf",
            "--manifest",
            "tests/test262-class-private-generator-methods.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CLASS_PRIVATE_GENERATOR_METHODS_PROFILE_SHA256
        );

        let profile = super::OxideProfile::load(Path::new(
            "tests/test262-class-private-generator-methods.conf",
        ))
        .unwrap();
        let positive = Path::new(
            "test/language/statements/class/elements/gen-private-method/yield-spread-arr-single.js",
        );
        for feature in [
            "class",
            "class-fields-private",
            "class-fields-public",
            "class-methods-private",
            "class-static-methods-private",
            "generators",
        ] {
            assert_eq!(
                profile.classify(positive, &[feature.to_owned()], false),
                None,
                "scoped profile did not admit {feature}"
            );
        }
        let audited_negative = Path::new(
            "test/language/statements/class/elements/gen-private-method/yield-as-binding-identifier.js",
        );
        assert_eq!(
            profile.classify(
                audited_negative,
                &["class-methods-private".to_owned(), "generators".to_owned(),],
                true,
            ),
            None
        );
        assert_eq!(
            profile
                .classify(
                    positive,
                    &["class-methods-private".to_owned(), "generators".to_owned(),],
                    true,
                )
                .unwrap()
                .outcome,
            "unsupported-negative-provenance"
        );

        let excluded_object_spread = Path::new(
            "test/language/statements/class/elements/gen-private-method/yield-spread-obj.js",
        );
        let excluded = profile
            .classify(
                excluded_object_spread,
                &[
                    "class-methods-private".to_owned(),
                    "generators".to_owned(),
                    "object-spread".to_owned(),
                ],
                false,
            )
            .unwrap();
        assert_eq!(excluded.outcome, "unsupported-feature");
        assert!(excluded.detail.ends_with("object-spread"));

        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        for feature in ["class-methods-private", "class-static-methods-private"] {
            assert_eq!(
                global
                    .classify(positive, &[feature.to_owned()], false)
                    .unwrap()
                    .outcome,
                "unsupported-feature",
                "global profile unexpectedly admitted {feature}"
            );
        }
        assert_eq!(
            global.classify(positive, &["generators".to_owned()], false),
            None,
            "global profile should admit the globally audited generators feature"
        );

        for selection in [
            ["--all", ""],
            ["--test", positive.to_str().unwrap()],
            ["--manifest", "tests/test262-class-generator-methods.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-class-private-generator-methods.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_promise_race_try_with_resolvers_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-promise-race-try-with-resolvers.conf",
            "--manifest",
            "tests/test262-promise-race-try-with-resolvers.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PROMISE_RACE_TRY_WITH_RESOLVERS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Promise/race/length.js"],
            ["--manifest", "tests/test262-promise-constructor-jobs.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-promise-race-try-with-resolvers.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_promise_finally_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-promise-finally.conf",
            "--manifest",
            "tests/test262-promise-finally.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PROMISE_FINALLY_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/built-ins/Promise/prototype/finally/length.js",
            ],
            [
                "--manifest",
                "tests/test262-promise-race-try-with-resolvers.txt",
            ],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-promise-finally.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_promise_all_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-promise-all.conf",
            "--manifest",
            "tests/test262-promise-all.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PROMISE_ALL_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Promise/all/length.js"],
            ["--manifest", "tests/test262-promise-finally.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-promise-all.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_promise_all_settled_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-promise-all-settled.conf",
            "--manifest",
            "tests/test262-promise-all-settled.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PROMISE_ALL_SETTLED_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Promise/allSettled/length.js"],
            ["--manifest", "tests/test262-promise-any.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-promise-all-settled.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_promise_any_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-promise-any.conf",
            "--manifest",
            "tests/test262-promise-any.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PROMISE_ANY_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Promise/any/length.js"],
            ["--manifest", "tests/test262-promise-all-settled.txt"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-promise-any.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_flat_array_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-binding-flat.conf",
            "--manifest",
            "tests/test262-array-binding-flat.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/ary-name-iter-val.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-binding-flat.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_nested_array_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-binding-nested.conf",
            "--manifest",
            "tests/test262-array-binding-nested.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/ary-ptrn-elem-ary-elem-iter.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-binding-nested.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_flat_array_assignment_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-assignment-flat.conf",
            "--manifest",
            "tests/test262-array-assignment-flat.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/assignment/dstr/array-empty-val-array.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-assignment-flat.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_catch_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-catch-binding.conf",
            "--manifest",
            "tests/test262-catch-binding.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CATCH_BINDING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/try/dstr/obj-ptrn-empty.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-catch-binding.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_identifier_rest_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-identifier-rest.conf",
            "--manifest",
            "tests/test262-identifier-rest.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_IDENTIFIER_REST_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/language/rest-parameters/rest-index.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-identifier-rest.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_identifier_defaults_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-identifier-defaults.conf",
            "--manifest",
            "tests/test262-identifier-defaults.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/function/dflt-params-ref-prior.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-identifier-defaults.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_parameter_direct_eval_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-parameter-direct-eval.conf",
            "--manifest",
            "tests/test262-parameter-direct-eval.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PARAMETER_DIRECT_EVAL_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/function-code/eval-param-env-with-computed-key.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-parameter-direct-eval.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_parameter_binding_patterns_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-parameter-binding-patterns.conf",
            "--manifest",
            "tests/test262-parameter-binding-patterns.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PARAMETER_BINDING_PATTERNS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/function/dstr/obj-ptrn-empty.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-parameter-binding-patterns.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_parameter_expression_binding_patterns_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-parameter-expression-binding-patterns.conf",
            "--manifest",
            "tests/test262-parameter-expression-binding-patterns.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PARAMETER_EXPRESSION_BINDING_PATTERNS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/function/dstr/obj-ptrn-id-init.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-parameter-expression-binding-patterns.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_object_assignment_profiles_are_bound_to_their_pinned_manifests() {
        for (cohort, expected_hash) in [
            ("flat", TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256),
            ("nested", TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256),
            ("rest", TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256),
        ] {
            let profile = format!("tests/test262-object-assignment-{cohort}.conf");
            let manifest = format!("tests/test262-object-assignment-{cohort}.txt");
            let invocation = parse(&[
                "--suite",
                "suite",
                "--oxide-profile",
                &profile,
                "--manifest",
                &manifest,
                "--report",
                "report.tsv",
            ])
            .unwrap();
            let Invocation::Coordinator(options) = invocation else {
                panic!("coordinator arguments selected another invocation");
            };
            assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);

            for selection in [
                ["--all", ""],
                [
                    "--test",
                    "test/language/expressions/assignment/dstr/obj-empty-obj.js",
                ],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments = vec!["--suite", "suite", "--oxide-profile", profile.as_str()];
                arguments.push(selection[0]);
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    #[test]
    fn scoped_object_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-object-binding.conf",
            "--manifest",
            "tests/test262-object-binding.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_OBJECT_BINDING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/obj-ptrn-empty.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-object-binding.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_object_rest_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-object-rest-binding.conf",
            "--manifest",
            "tests/test262-object-rest-binding.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_OBJECT_REST_BINDING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/obj-ptrn-rest-getter.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-object-rest-binding.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_array_buffer_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-buffer.conf",
            "--manifest",
            "tests/test262-array-buffer.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_BUFFER_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/ArrayBuffer/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-buffer.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_data_view_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-data-view.conf",
            "--manifest",
            "tests/test262-data-view.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_DATA_VIEW_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/DataView/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-data-view.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_typed_array_core_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-typed-array-core.conf",
            "--manifest",
            "tests/test262-typed-array-core.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_TYPED_ARRAY_CORE_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/TypedArray/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-typed-array-core.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_uint8array_codec_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-uint8array-codecs.conf",
            "--manifest",
            "tests/test262-uint8array-codecs.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_UINT8ARRAY_CODECS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Uint8Array/fromBase64/results.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-uint8array-codecs.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn uint8array_codec_global_admission_profiles_require_the_tag_manifest_or_all() {
        for (profile, expected_hash) in [
            (
                "tests/test262-uint8array-codecs-global-parent.conf",
                TEST262_UINT8ARRAY_CODECS_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-uint8array-codecs-global-candidate.conf",
                TEST262_UINT8ARRAY_CODECS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            for selection in [
                ["--manifest", "tests/test262-uint8array-codecs.txt"],
                ["--all", ""],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
            }

            for selection in [
                ["--test", "test/built-ins/Uint8Array/fromBase64/results.js"],
                ["--manifest", "tests/test262-typed-array-core.txt"],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                arguments.push(selection[1]);
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    #[test]
    fn scoped_resizable_arraybuffer_profile_is_bound_to_its_activation_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-resizable-arraybuffer.conf",
            "--manifest",
            "tests/test262-resizable-arraybuffer.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_RESIZABLE_ARRAYBUFFER_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/built-ins/ArrayBuffer/prototype/resize/resize.js",
            ],
            [
                "--manifest",
                "tests/test262-resizable-arraybuffer-universe.txt",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-resizable-arraybuffer.conf",
                selection[0],
            ];
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn resizable_arraybuffer_global_profiles_require_the_universe_manifest_or_all() {
        for (profile, expected_hash) in [
            (
                "tests/test262-resizable-arraybuffer-global-parent.conf",
                TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-resizable-arraybuffer-global-candidate.conf",
                TEST262_RESIZABLE_ARRAYBUFFER_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            for selection in [
                [
                    "--manifest",
                    "tests/test262-resizable-arraybuffer-universe.txt",
                ],
                ["--all", ""],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
            }

            for selection in [
                [
                    "--test",
                    "test/built-ins/ArrayBuffer/prototype/resize/resize.js",
                ],
                ["--manifest", "tests/test262-resizable-arraybuffer.txt"],
                [
                    "--manifest",
                    "tests/test262-resizable-arraybuffer-reason-only.txt",
                ],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                arguments.push(selection[1]);
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    fn assert_computed_property_names_profile_binding(profile: &str, expected_hash: &str) {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            profile,
            "--manifest",
            "tests/test262-computed-property-names-universe.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/object/computed-property-name.js",
            ],
            [
                "--manifest",
                "tests/test262-computed-property-names-activation.txt",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }

        assert!(
            parse_error(&[
                "--suite",
                "suite",
                "--oxide-profile",
                profile,
                "--report",
                "report.tsv",
            ])
            .contains("select exactly one")
        );
    }

    #[test]
    fn computed_property_names_candidate_requires_its_pinned_universe_manifest() {
        assert_computed_property_names_profile_binding(
            "tests/test262-computed-property-names.conf",
            TEST262_COMPUTED_PROPERTY_NAMES_CANDIDATE_PROFILE_SHA256,
        );
    }

    #[test]
    fn computed_property_names_parent_requires_its_pinned_universe_manifest() {
        assert_computed_property_names_profile_binding(
            "tests/test262-computed-property-names-parent.conf",
            TEST262_COMPUTED_PROPERTY_NAMES_PARENT_PROFILE_SHA256,
        );
    }

    fn assert_tag_transition_profile_binding(
        profile: &str,
        expected_hash: &str,
        accepted_manifests: &[&str],
        rejected_manifest: &str,
        rejected_test: &str,
    ) {
        for manifest in accepted_manifests {
            let arguments = [
                "--suite",
                "suite",
                "--oxide-profile",
                profile,
                "--manifest",
                manifest,
                "--report",
                "report.tsv",
            ];
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
        }

        let arguments = [
            "--suite",
            "suite",
            "--oxide-profile",
            profile,
            "--all",
            "--report",
            "report.tsv",
        ];
        let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);

        for selection in [
            ["--test", rejected_test],
            ["--manifest", rejected_manifest],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }

        assert!(
            parse_error(&[
                "--suite",
                "suite",
                "--oxide-profile",
                profile,
                "--report",
                "report.tsv",
            ])
            .contains("select exactly one")
        );
    }

    #[test]
    fn rest_parameters_profiles_require_their_pinned_universe_manifest() {
        for (profile, expected_hash) in [
            (
                "tests/test262-rest-parameters-parent.conf",
                TEST262_REST_PARAMETERS_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-rest-parameters-candidate.conf",
                TEST262_REST_PARAMETERS_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            assert_tag_transition_profile_binding(
                profile,
                expected_hash,
                &["tests/test262-rest-parameters-universe.txt"],
                "tests/test262-rest-parameters-activation.txt",
                "test/language/expressions/function/rest-param-strict-body.js",
            );
        }
    }

    #[test]
    fn default_parameters_profiles_require_their_pinned_universe_manifest() {
        for (profile, expected_hash) in [
            (
                "tests/test262-default-parameters-parent.conf",
                TEST262_DEFAULT_PARAMETERS_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-default-parameters-candidate.conf",
                TEST262_DEFAULT_PARAMETERS_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            assert_tag_transition_profile_binding(
                profile,
                expected_hash,
                &[
                    "tests/test262-default-parameters-universe.txt",
                    "tests/test262-default-parameters-strict-body.txt",
                ],
                "tests/test262-identifier-defaults.txt",
                "test/language/expressions/function/dflt-params.js",
            );
        }

        assert_tag_transition_profile_binding(
            "tests/test262-default-parameters-global-candidate.conf",
            TEST262_DEFAULT_PARAMETERS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            &[
                "tests/test262-default-parameters-universe.txt",
                "tests/test262-default-parameters-strict-body.txt",
            ],
            "tests/test262-identifier-defaults.txt",
            "test/language/expressions/function/dflt-params.js",
        );
    }

    #[test]
    fn data_view_global_profiles_require_their_pinned_universe_manifest() {
        for (profile, expected_hash) in [
            (
                "tests/test262-data-view-global-parent.conf",
                TEST262_DATA_VIEW_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-data-view-global-candidate.conf",
                TEST262_DATA_VIEW_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            assert_tag_transition_profile_binding(
                profile,
                expected_hash,
                &["tests/test262-data-view-universe.txt"],
                "tests/test262-data-view.txt",
                "test/built-ins/DataView/is-a-constructor.js",
            );
        }
    }

    #[test]
    fn object_rest_global_profiles_require_their_pinned_manifests() {
        for (profile, expected_hash) in [
            (
                "tests/test262-object-rest-global-parent.conf",
                TEST262_OBJECT_REST_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-object-rest-global-candidate.conf",
                TEST262_OBJECT_REST_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            assert_tag_transition_profile_binding(
                profile,
                expected_hash,
                &[
                    "tests/test262-object-rest-universe.txt",
                    "tests/test262-object-rest-companion.txt",
                ],
                "tests/test262-object-rest-binding.txt",
                "test/language/statements/variable/dstr/obj-ptrn-rest-getter.js",
            );
        }
    }

    #[test]
    fn weak_collections_global_profiles_require_their_pinned_universe_manifest() {
        for (profile, expected_hash) in [
            (
                "tests/test262-weak-collections-global-parent.conf",
                TEST262_WEAK_COLLECTIONS_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-weak-collections-global-candidate.conf",
                TEST262_WEAK_COLLECTIONS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            assert_tag_transition_profile_binding(
                profile,
                expected_hash,
                &["tests/test262-weak-collections-global-universe.txt"],
                "tests/test262-weak-collections.txt",
                "test/built-ins/WeakMap/length.js",
            );
        }
    }

    #[test]
    fn computed_property_names_global_profiles_require_the_universe_manifest_or_all() {
        for (profile, expected_hash) in [
            (
                "tests/test262-computed-property-names-global-parent.conf",
                TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-computed-property-names-global-candidate.conf",
                TEST262_COMPUTED_PROPERTY_NAMES_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            for selection in [
                [
                    "--manifest",
                    "tests/test262-computed-property-names-universe.txt",
                ],
                ["--all", ""],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
            }

            for selection in [
                [
                    "--test",
                    "test/language/expressions/object/computed-property-name.js",
                ],
                [
                    "--manifest",
                    "tests/test262-computed-property-names-activation.txt",
                ],
                [
                    "--manifest",
                    "tests/test262-computed-property-names-reason-only.txt",
                ],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                arguments.push(selection[1]);
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }

            assert!(
                parse_error(&[
                    "--suite",
                    "suite",
                    "--oxide-profile",
                    profile,
                    "--report",
                    "report.tsv",
                ])
                .contains("select exactly one")
            );
        }
    }

    #[test]
    fn scoped_map_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-map.conf",
            "--manifest",
            "tests/test262-map.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_MAP_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Map/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-map.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_set_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-set.conf",
            "--manifest",
            "tests/test262-set.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_SET_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Set/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-set.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_weak_collections_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-weak-collections.conf",
            "--manifest",
            "tests/test262-weak-collections.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_WEAK_COLLECTIONS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/WeakMap/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-weak-collections.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_symbol_protocol_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-symbol-protocols.conf",
            "--manifest",
            "tests/test262-symbol-protocols.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Symbol/iterator/prop-desc.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-symbol-protocols.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_regexp_builtins_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-regexp-builtins.conf",
            "--manifest",
            "tests/test262-regexp-builtins.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_REGEXP_BUILTINS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/RegExp/escape/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-regexp-builtins.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_generator_destructuring_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-generator-destructuring.conf",
            "--manifest",
            "tests/test262-generator-destructuring.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_GENERATOR_DESTRUCTURING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/generators/yield-as-literal.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-generator-destructuring.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_iterator_helpers_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-iterator-helpers.conf",
            "--manifest",
            "tests/test262-iterator-helpers.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ITERATOR_HELPERS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/built-ins/Iterator/prototype/map/callable.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-iterator-helpers.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn iterator_helpers_global_admission_profiles_require_the_manifest_or_all() {
        for (profile, expected_hash) in [
            (
                "tests/test262-iterator-helpers-global-parent.conf",
                TEST262_ITERATOR_HELPERS_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-iterator-helpers-global-candidate.conf",
                TEST262_ITERATOR_HELPERS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            for selection in [
                ["--manifest", "tests/test262-iterator-helpers-global.txt"],
                ["--all", ""],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
            }

            for selection in [
                [
                    "--test",
                    "test/built-ins/Iterator/prototype/map/callable.js",
                ],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                arguments.push(selection[1]);
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }

        assert_eq!(
            parse_error(&[
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-iterator-helpers-global-candidate.conf",
                "--all",
                "--manifest",
                "tests/test262-iterator-helpers-global.txt",
                "--report",
                "report.tsv",
            ]),
            "select exactly one of --all, --manifest, or one-or-more --test"
        );
    }

    #[test]
    fn global_this_transition_profiles_are_bound_to_the_activation_manifest() {
        for (profile, expected_hash) in [
            (
                "tests/test262-global-this-parent.conf",
                TEST262_GLOBAL_THIS_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-global-this-candidate.conf",
                TEST262_GLOBAL_THIS_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            let invocation = parse(&[
                "--suite",
                "suite",
                "--oxide-profile",
                profile,
                "--manifest",
                "tests/test262-global-this-activation.txt",
                "--report",
                "report.tsv",
            ])
            .unwrap();
            let Invocation::Coordinator(options) = invocation else {
                panic!("coordinator arguments selected another invocation");
            };
            assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);

            for selection in [
                ["--all", ""],
                ["--test", "test/built-ins/global/global-object.js"],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    #[test]
    fn global_this_global_admission_profiles_require_the_tag_manifest_or_all() {
        for (profile, expected_hash) in [
            (
                "tests/test262-global-this-global-parent.conf",
                TEST262_GLOBAL_THIS_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-global-this-global-candidate.conf",
                TEST262_GLOBAL_THIS_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            for selection in [
                ["--manifest", "tests/test262-global-this.txt"],
                ["--all", ""],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
            }

            for selection in [
                ["--test", "test/built-ins/global/global-object.js"],
                ["--manifest", "tests/test262-global-this-activation.txt"],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                arguments.push(selection[1]);
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    #[test]
    fn promise_global_admission_profiles_require_the_tag_manifest_or_all() {
        for (profile, expected_hash) in [
            (
                "tests/test262-promise-global-parent.conf",
                TEST262_PROMISE_GLOBAL_PARENT_PROFILE_SHA256,
            ),
            (
                "tests/test262-promise-global-candidate.conf",
                TEST262_PROMISE_GLOBAL_CANDIDATE_PROFILE_SHA256,
            ),
        ] {
            for selection in [
                ["--manifest", "tests/test262-promise-global.txt"],
                ["--all", ""],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);
            }

            for selection in [
                ["--test", "test/built-ins/Promise/any/name.js"],
                ["--manifest", "tests/test262-promise-global-activation.txt"],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments =
                    vec!["--suite", "suite", "--oxide-profile", profile, selection[0]];
                arguments.push(selection[1]);
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    #[test]
    fn scoped_iterator_sequencing_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-iterator-sequencing.conf",
            "--manifest",
            "tests/test262-iterator-sequencing.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ITERATOR_SEQUENCING_PROFILE_SHA256
        );
        let positive = Path::new("test/built-ins/Iterator/concat/single-argument.js");
        let global = super::OxideProfile::load(Path::new("compat/test262-oxide.conf")).unwrap();
        assert_eq!(
            global.classify(positive, &["iterator-sequencing".to_owned()], false),
            None,
            "global profile should admit authenticated Iterator sequencing"
        );

        for selection in [
            ["--all", ""],
            ["--test", positive.to_str().unwrap()],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-iterator-sequencing.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_proxy_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-proxy.conf",
            "--manifest",
            "tests/test262-proxy.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_PROXY_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Proxy/constructor.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-proxy.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_optional_chaining_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-optional-chaining.conf",
            "--manifest",
            "tests/test262-optional-chaining.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_OPTIONAL_CHAINING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/optional-chaining/member-expression.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-optional-chaining.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }
}
