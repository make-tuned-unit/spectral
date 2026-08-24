#!/usr/bin/env python3
"""Discriminative probe: given each question's actual FTS seed (rank-1 key from
the C arm), does fingerprint distance rank gold evidence turns above non-gold?
AUC per question, averaged; content-only vs enriched fingerprints."""
import json,sqlite3,glob,math,sys,statistics as st
def load_fp(db):
    c=sqlite3.connect(db)
    rows=c.execute("select m.key,s.entity_density,s.action_type,s.decision_polarity,s.causal_depth,s.emotional_valence,s.temporal_specificity,s.novelty from memory_spectrogram s join memories m on m.id=s.memory_id").fetchall()
    return {r[0].replace('answer_',''):(r[2],[r[1],r[3],r[4],r[5],r[6],r[7]]) for r in rows}
def dist(a,b): return math.sqrt(sum((x-y)**2 for x,y in zip(a[1],b[1])))
def auc(pos,neg):
    if not pos or not neg: return None
    w=0;n=0
    for p in pos:
        for q in neg:
            n+=1
            if p<q: w+=1
            elif p==q: w+=0.5
    return w/n
res={}
for s in ('3','7'):
    ds={r['question_id']:r for r in json.load(open(f'dataset_s{s}.json'))}
    seeds={}
    for l in open(f'C_s{s}.jsonl'):
        d=json.loads(l); k=d.get('retrieved_keys') or [None]; seeds[d['question_id']]=(k[0] or '').replace('answer_','')
    for arm in ('content','new'):
        db=glob.glob(f'probe-{arm}-s{s}/brain_*/memory.db')[0]
        fp=load_fp(db)
        aucs=[]; aucs_same_at=[]; aucs_gold_seed=[]
        for qid,q in ds.items():
            gold=set()
            for sid,turns in zip(q['haystack_session_ids'],q['haystack_sessions']):
                for i,t in enumerate(turns):
                    if t.get('has_answer'): gold.add(f"{sid.replace('answer_','')}:turn:{i}:{t['role']}")
            gold={g for g in gold if g in fp}
            if not gold: continue
            seed=seeds.get(qid)
            if seed not in fp: continue
            others=[k for k in fp if k!=seed]
            dpos=[dist(fp[seed],fp[k]) for k in others if k in gold]
            dneg=[dist(fp[seed],fp[k]) for k in others if k not in gold]
            a=auc(dpos,dneg)
            if a is not None: aucs.append(a)
            # gold-as-seed: does the rest of the gold cluster around one gold turn?
            g0=sorted(gold)[0]
            if len(gold)>=2:
                dpos=[dist(fp[g0],fp[k]) for k in gold if k!=g0]
                dneg=[dist(fp[g0],fp[k]) for k in fp if k not in gold and k!=g0]
                a=auc(dpos,dneg)
                if a is not None: aucs_gold_seed.append(a)
        res[(s,arm)]=(st.mean(aucs),len(aucs),st.mean(aucs_gold_seed),len(aucs_gold_seed))
        print(f"s{s} {arm:8} AUC(fts seed -> gold vs non-gold)={st.mean(aucs):.3f} (n={len(aucs)})   AUC(gold seed -> other gold vs non-gold)={st.mean(aucs_gold_seed):.3f} (n={len(aucs_gold_seed)})")
