import re, subprocess, sys
base, head, path, out = sys.argv[1:5]
txt = subprocess.run(['git','diff','--word-diff=plain','-U0',base,head,'--',path],capture_output=True,text=True,encoding='utf-8').stdout
res=[]; cur=None
for line in txt.split('\n'):
    m=re.match(r'^@@ -\d+(?:,\d+)? \+(\d+)',line)
    if m: cur=int(m.group(1)); continue
    if cur is None or line.startswith(('diff ','index ','--- ','+++ ')): continue
    spans=[(mm.start(),mm.end()) for mm in re.finditer(r'\[-.*?-\]|\{\+.*?\+\}',line,flags=re.S)]
    if spans:
        merged=[]
        for a,b in spans:
            if merged and a-merged[-1][1]<160: merged[-1]=(merged[-1][0],b)
            else: merged.append((a,b))
        parts=[line[max(0,a-80):min(len(line),b+80)] for a,b in merged]
        res.append(f"L{cur}: " + "  …  ".join(parts))
    cur+=1
open(out,'w',encoding='utf-8').write('\n\n'.join(res))
print(len(res),'rows', sum(len(r) for r in res),'chars')
