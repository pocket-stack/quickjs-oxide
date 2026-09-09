const fs = require("node:fs");
const [path, variantsText, expectedSummary, quickjs, test262, patch, config,
  metadataHash, profileHash, negativeDiagnosticsHash,
  negativeDiagnosticExemptionsHash, engineHash, mode, expectedSchemaText] =
  process.argv.slice(2);
const lines = fs.readFileSync(path, "utf8").trimEnd().split("\n");
const records = lines.map((line) => JSON.parse(line));
const variants = Number(variantsText);
const expectedSchema = Number(expectedSchemaText);
if (records.length !== variants + 2) process.exit(2);
const metadata = records[0];
if (metadata.kind !== "metadata" || metadata.schema !== expectedSchema ||
    ![3, 4, 5].includes(metadata.schema) ||
    metadata.quickjs !== quickjs || metadata.test262 !== test262 ||
    metadata.test262_patch_sha256 !== patch ||
    metadata.test262_config_sha256 !== config ||
    metadata.test262_metadata_sha256 !== metadataHash ||
    metadata.oxide_profile_sha256 !== profileHash ||
    (metadata.schema >= 4 &&
      metadata.negative_diagnostics_sha256 !== negativeDiagnosticsHash) ||
    (metadata.schema === 5 &&
      metadata.negative_diagnostic_exemptions_sha256 !==
        negativeDiagnosticExemptionsHash) ||
    metadata.engine_semantics_sha256 !== engineHash ||
    metadata.profile !== "test262-canonical-classified-v2" ||
    metadata.mode !== mode) process.exit(3);
const summary = records.at(-1);
if (summary.kind !== "summary") process.exit(4);
const actualSummary = Object.entries(summary.outcomes)
  .map(([name, count]) => `${name}=${count}`).join(" ");
if (actualSummary !== expectedSummary) process.exit(5);
const fieldsV3 = [
  "path", "variant", "flags", "features", "expected_phase",
  "expected_type", "outcome", "actual_phase", "actual_type", "detail",
];
const fields = metadata.schema === 3 ? fieldsV3 : [
  ...fieldsV3,
  "expected_rule", "expected_message", "expected_line", "expected_column",
  "location_policy", "actual_line", "actual_column",
];
function escapeField(value) {
  let output = "";
  for (const character of value) {
    if (character === "\\") output += "\\\\";
    else if (character === "\t") output += "\\t";
    else if (character === "\n") output += "\\n";
    else if (character === "\r") output += "\\r";
    else if (/\p{Cc}/u.test(character)) {
      output += "\\u" + character.codePointAt(0).toString(16).padStart(4, "0");
    } else output += character;
  }
  return output;
}
for (const record of records.slice(1, -1)) {
  if (record.kind !== "result" || fields.some((field) => typeof record[field] !== "string")) {
    process.exit(6);
  }
  process.stdout.write(fields.map((field) => escapeField(record[field])).join("\t") + "\n");
}
