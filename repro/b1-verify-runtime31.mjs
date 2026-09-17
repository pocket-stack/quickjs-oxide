// Differential verification of the 31 script-goal unaudited runtime negatives.
// Oracle: pinned qjs with harness compiled as separate scripts (-I), authored
// strict via prefix — identical invocation shape to the parse-contract audit.
// Oxide: the gate's own --worker-one (the binary that enforces contracts).
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
const SUITE="/var/tmp/oss/qjs-oracle-cache/quickjs-2026-06-04/test262";
const QJS="/var/tmp/oss/qjs-oracle-cache/quickjs-2026-06-04/qjs";
const RT="target/release/run-test262";
const A="dev-support/test262/admissions.tsv";
const AS=spawnSync("grep",["^admissions_sha256=","dev-support/test262/current.conf"],{encoding:"utf8"}).stdout.trim().split("=")[1];
const scout=spawnSync("git",["show","fleet/task-873:repro/evidence/neg-rt-worker.txt"],{encoding:"utf8"}).stdout.trimEnd().split("\n");
function frame(text){
  for(const raw of text.split("\n")){
    let f=raw.trim();
    if(!f.startsWith("at ")) continue;
    f=f.slice(3);
    if(f.endsWith(")")&&f.includes(" (")) f=f.slice(f.lastIndexOf(" (")+2,-1);
    const m=f.match(/^(.+):([1-9][0-9]*):([1-9][0-9]*)$/);
    if(m&&m[1]) return [Number(m[2]),Number(m[3])];
  }
  return [];
}
let n=0, mism=0;
for(const line of scout){
  const [p,v,outcome,metaPhase,metaType]=line.split("\t");
  if(outcome!=="pass") continue;
  n++;
  const src=readFileSync(`${SUITE}/${p}`,"utf8");
  const authored=v==="strict" ? `"use strict";\n${src}` : src;
  const q=spawnSync(QJS,["-I",`${SUITE}/harness/assert.js`,"-I",`${SUITE}/harness/sta.js`,"--script","-e",authored],{encoding:"utf8",timeout:15000});
  const text=`${q.stdout}${q.stderr}`.replaceAll("\r\n","\n");
  const el=text.split("\n").find(x=>/^[A-Za-z_$][\w$]*Error: /.test(x));
  let qtype,qmsg,ql,qc;
  if(el){ const at=el.indexOf(": "); qtype=el.slice(0,at); qmsg=el.slice(at+2); [ql,qc]=frame(text); }
  else {
    // non-Error thrown object: QuickJS dumps a bare object literal.
    const m=text.match(/^\{ message: ("(?:[^"\\]|\\.)*"|) \}\s*$/m);
    if(!m){ console.log("UNPARSEABLE",p,v,JSON.stringify(text.slice(0,160))); mism++; continue; }
    qtype="Test262Error";
    let raw=m[1]; if(raw.startsWith('"')) raw=raw.slice(1,-1);
    qmsg=raw.replace(/\\"/g,'"').replace(/\\\\/g,"\\");
  }
  const o=spawnSync(RT,["--worker-one","--suite",SUITE,"--test",p,"--admissions",A,"--admissions-sha256",AS,"--variant",v],{encoding:"utf8",timeout:15000});
  const of=`${o.stdout}${o.stderr}`.trim().split("\n")[0].split("\t");
  // outcome actual_phase actual_type [detail [line [column]]]
  const ophase=of[1], otype=of[2], omsg=of[3] ?? "", ol=of[4], oc=of[5];
  const oln=ol?Number(ol):undefined, ocn=oc?Number(oc):undefined;
  const qlcv=ql===undefined?undefined:ql, qccv=qc;
  const ok = ophase==="runtime" && otype===qtype && omsg===qmsg && oln===qlcv && ocn===qccv && otype===metaType;
  if(!ok){ mism++; console.log(`MISMATCH ${p} ${v}\n  qjs   ={${qtype}|${qmsg}|${qlcv}:${qccv}}\n  oxide ={${otype}|${omsg}|${oln}:${ocn}} metaType=${metaType} phase=${ophase}`); }
}
console.log(`runtime rows=${n} mismatches=${mism}`);
