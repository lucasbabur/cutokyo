import { strict as assert } from "node:assert";
import { mkdtempSync, writeFileSync, chmodSync, symlinkSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createTrustedKeyboard } from "/home/lucas/Developer/personal/cutokyo-community/ui/tests/tauri/trusted-keyboard.ts";
const root=mkdtempSync(join(tmpdir(),"cutokyo-cli-validation-"));
const regular=join(root,"regular-cli");writeFileSync(regular,"#!/bin/sh\nexit 99\n");chmodSync(regular,0o700);
const alias=join(root,"alias-cli");symlinkSync(regular,alias);
const directory=join(root,"directory-cli");mkdirSync(directory);
const noexec=join(root,"non-executable-cli");writeFileSync(noexec,"not executable");chmodSync(noexec,0o600);
const env={CUTOKYO_NATIVE_APPLICATION:"/private/package/usr/bin/cutokyo-desktop",CUTOKYO_NATIVE_COMPOSITOR_RUNTIME:"/host/compositor-runtime",HYPRLAND_INSTANCE_SIGNATURE:"synthetic",XDG_RUNTIME_DIR:"/private/runtime"};
const results=[];
try {
for(const [name,path,valid] of [["resolved regular executable",regular,true],["symlink",alias,false],["directory",directory,false],["non executable",noexec,false],["missing",join(root,"missing"),false]] as const) {
let accepted=false;let error="";
try {createTrustedKeyboard({...env,CUTOKYO_NATIVE_COMPOSITOR_CLI:path});accepted=true;}catch(e){error=String(e);}
assert.equal(accepted,valid,name);results.push({name,expectedAccepted:valid,actualAccepted:accepted,error});
}
console.log(JSON.stringify({scope:"Real filesystem prerequisite validation only; never calls send or executes any CLI",results},null,2));
}finally{rmSync(root,{recursive:true,force:true});}
