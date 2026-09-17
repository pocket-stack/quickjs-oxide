// 17 native-Error script-runtime rows: no custom includes, so compare plain
// qjs --script -e authored vs oxide -e authored (the audit tool's own channel),
// strict via prefix (same coordinate convention as parse contracts).
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
const SUITE="/var/tmp/oss/qjs-oracle-cache/quickjs-2026-06-04/test262";
const QJS="/var/tmp/oss/qjs-oracle-cache/quickjs-2026-06-04/qjs";
const OX="target/release/qjs";
const scout=spawnSync("git",["show","fleet/task-873:repro/evidence/neg-rt-worker.txt"],{encoding:"utf8"}).stdout.trimEnd().split("\n");
function frame(t){for(const r0 of t.split("\n")){let f=r0.trim();if(!f.startsWith("at "))continue;f=f.slice(3);if(f.endsWith(")")&&f.includes(" ("))f=f.slice(f.lastIndexOf(" (")+2,-1);const m=f.match(/^(.+):([1-9][0-9]*):([1-9][0-9]*)$/);if(m&&m[1])return[Number(m[2]),Number(m[3])];}return[];}
function diag(r){const t=`${r.stdout}${r.stderr}`.replaceAll("\r\n","\n");const el=t.split("\n").find(x=>/^[A-Za-z_$][\w$]*Error: /.test(x));if(!el)return null;const i=el.indexOf(": ");const[l,c]=frame(t);return{type:el.slice(0,i),msg:el.slice(i+2),line:l,col:c,status:r.status};}
let n=0,bad=0;
for(const line of scout){
  const [p,v,outcome,phase,type,msg,l,c]=line.split("\t");
  if(outcome!=="pass"||type==="Test262Error") continue;
  n++;
  const src=readFileSync(`${SUITE}/${p}`,"utf8");
  const authored=v==="strict"?`"use strict";\n${src}`:src;
  const q=diag(spawnSync(QJS,["--script","-e",authored],{encoding:"utf8",timeout:15000}));
  const o=diag(spawnSync(OX,["-e",authored],{encoding:"utf8",timeout:15000}));
  const ok=q&&o&&q.type===o.type&&q.msg===o.msg&&q.line===o.line&&q.col===o.col&&q.type===type&&q.msg===msg&&q.line===(l?+l:undefined);
  if(!ok){bad++;console.log(`MISMATCH ${p} ${v}\n qjs=${JSON.stringify(q)}\n oxide=${JSON.stringify(o)} gate={${type}|${msg}|${l}:${c}}`);}
}
console.log(`native runtime rows=${n} mismatches=${bad}`);
