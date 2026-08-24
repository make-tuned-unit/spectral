#!/usr/bin/env python3
import json,sys,math
from itertools import combinations
def load(paths):
    rows={}
    for p in paths:
        try:
            for l in open(p):
                try: d=json.loads(l)
                except json.JSONDecodeError: continue
                if d.get('evidence_turns_total') and d.get('evidence_turns_retrieved') is not None:
                    rows[d['question_id']]=(d['evidence_turns_retrieved'],d['evidence_turns_total'])
        except FileNotFoundError: pass
    return rows
def micro(r,ids): 
    a=sum(r[i][0] for i in ids); b=sum(r[i][1] for i in ids); return a/b if b else float('nan')
def wilcoxon(x,y):
    # paired signed-rank, normal approx with tie correction, two-sided
    d=[a-b for a,b in zip(x,y) if a!=b]
    n=len(d)
    if n<10: return float('nan'),n
    ranks=sorted(range(n),key=lambda i:abs(d[i]))
    r=[0]*n; i=0
    while i<n:
        j=i
        while j+1<n and abs(d[ranks[j+1]])==abs(d[ranks[i]]): j+=1
        avg=(i+j)/2+1
        for k in range(i,j+1): r[ranks[k]]=avg
        i=j+1
    wp=sum(r[i] for i in range(n) if d[i]>0)
    mean=n*(n+1)/4; var=n*(n+1)*(2*n+1)/24
    # tie correction
    from collections import Counter
    c=Counter(abs(v) for v in d)
    var-=sum(t**3-t for t in c.values())/48
    z=(wp-mean)/math.sqrt(var)
    p=math.erfc(abs(z)/math.sqrt(2))
    return p,n
arms=sys.argv[1:] or ['C','S0','Snew','Sold']
data={a:load([f'{a}_s3.jsonl',f'{a}_s7.jsonl']) for a in arms}
data={a:v for a,v in data.items() if v}
print({a:len(v) for a,v in data.items()})
for a,b in combinations(data,2):
    ids=sorted(set(data[a])&set(data[b]))
    ma,mb=micro(data[a],ids),micro(data[b],ids)
    p,n=wilcoxon([data[b][i][0] for i in ids],[data[a][i][0] for i in ids])
    up=sum(1 for i in ids if data[b][i][0]>data[a][i][0]); dn=sum(1 for i in ids if data[b][i][0]<data[a][i][0])
    print(f"{b} vs {a}: n={len(ids)} micro {ma*100:.2f}% -> {mb*100:.2f}% ({(mb-ma)*100:+.2f}pp)  q up/down {up}/{dn}  Wilcoxon p={p:.4f} (n_nonzero={n})")
