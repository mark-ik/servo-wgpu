from pathlib import Path
import subprocess,os,time,json,hashlib,sys
p=Path(__file__).resolve().parent
label=sys.argv[1]
env=os.environ.copy()
env['CARGO_HOME']=str(p/'cargo-home')
env['CARGO_TARGET_DIR']=str(p/'target-s6')
cmd=['cargo','test','--offline','--locked','--manifest-path',str(p/'probe-s6/Cargo.toml'),'--','--nocapture']
t=time.time()
r=subprocess.run(cmd,env=env,cwd=p,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
(p/(label+'.log')).write_bytes(r.stdout)
meta={'command':cmd,'exit_code':r.returncode,'elapsed':time.time()-t,'compiler':subprocess.check_output(['rustc','--version'],text=True).strip(),'engine':'Boa','renderer':'none','server':'host-supplied synchronous worker script loader; no HTTP server','features':'genet-scripted default-features=false','sources':{f:hashlib.sha256((p/f).read_bytes()).hexdigest() for f in ['probe-s6/Cargo.toml','probe-s6/Cargo.lock','probe-s6/src/lib.rs','source-s6/components/genet-scripted/document.rs','run_s6.py']}}
(p/(label+'-run.json')).write_text(json.dumps(meta,indent=2),encoding='utf-8')
print(r.returncode)
print(r.stdout.decode('utf-8',errors='replace')[-4200:])
