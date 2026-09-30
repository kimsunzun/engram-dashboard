import re,glob,sys
def is_hook(cmd):
    m=re.match(r'^("[^"]*"|\S+)\s+-c\s+"(.*)"\s*$',cmd or '')
    return bool(m) and m.group(2).startswith('bash ') and 'shell-snapshots' not in m.group(2)
def tk_target(cmd):
    m=re.search(r'/PID (\d+) /T /F\s*$',cmd or '')
    return int(m.group(1)) if m else None
TOT={'kills':0,'trials':0}
for fn in sorted(glob.glob('*.tsv')):
    lines=open(fn,encoding='utf-8',errors='replace').read().splitlines()
    root=int(re.search(r'root_pid=(\d+)',lines[0]).group(1))
    P=[]
    for l in lines[2:]:
        p=l.split('\t')
        P.append(dict(pid=int(p[0]),st=int(p[1]),ex=int(p[2]) if p[2] else 10**20,ppid=int(p[3]),exe=p[10],cmd=p[11]))
    claude=[x for x in P if x['ppid']==root and x['exe'].endswith('claude.exe')][0]
    tks=sorted([x for x in P if x['ppid']==claude['pid'] and x['exe'].lower().endswith('taskkill.exe')],key=lambda x:x['st'])
    escs=[]; last=-10**20
    for t in tks:
        if t['st']-last>3000*10**4: escs.append(t['st']-5*10**4)
        last=t['st']
    for esc in escs:
        TOT['trials']+=1
        tpass=esc+3000*10**4
        snap=[x for x in P if x['st']<=esc<x['ex']]
        births=[x for x in P if esc<x['st']<=tpass]
        known=snap+births
        killed=set()
        def alive(x,t): return x['st']<=t<x['ex'] and id(x) not in killed
        def find(pid,child):
            c=[k for k in known if k['pid']==pid and k['st']<=child['st']]
            return max(c,key=lambda k:k['st']) if c else None
        for rnd in range(3):
            cands=[x for x in births if alive(x,tpass)]
            dec=[]
            for C in cands:
                D=find(C['ppid'],C)
                if not D or D['pid']==root: continue
                if alive(D,tpass): continue
                if not (C['exe']==D['exe'] and (C['cmd']==D['cmd'] or is_hook(D['cmd']))): continue
                A=D; ok=True; steps=0
                while True:
                    par=find(A['ppid'],A)
                    if par is None: ok=False;break
                    if par['ppid']==root: cl=par;T=A;break
                    if alive(par,tpass): ok=False;break
                    A=par; steps+=1
                    if steps>16: ok=False;break
                if not ok or alive(T,tpass) or T is D and False: continue
                if T is D and C['cmd']!=D['cmd'] and False: pass
                if not (T['exe'].lower().split(chr(92))[-1]=='bash.exe' and is_hook(T['cmd'])): continue
                tkok=[t for t in births if t['ppid']==cl['pid'] and t['exe'].lower().endswith('taskkill.exe') and tk_target(t['cmd'])==T['pid']]
                if not tkok: continue
                dec.append((C,T,tkok))
            if not dec: break
            for C,T,tkok in dec:
                killed.add(id(C)); TOT['kills']+=1
                print(fn,'esc~%d'%esc,'round',rnd+1,'KILL',C['pid'],'+%.0f'%((C['st']-esc)/1e4),C['cmd'][-40:],'T',T['pid'],'tk>=T',all(t['st']>=T['st'] for t in tkok))
print(TOT)
