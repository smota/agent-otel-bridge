#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Informative fixture authoring, not an importer.

Expected outcomes are authored below, never obtained from either checker.
Run after reviewing SPEC and both implementations. Test keys are public fixtures.
"""
import copy
import hashlib
import hmac
import json
import pathlib
import struct

BASE=pathlib.Path(__file__).resolve().parent
TABLES=("sessions","invocations","tool_calls","usage","outcomes","git_evidence","coverage")
PK=dict(zip(TABLES,("session_key","invocation_key","tool_call_key","usage_key","outcome_key","git_evidence_key","coverage_key")))
SCHEMAS={t:json.loads((BASE/f"tables/{t}.schema.json").read_text()) for t in TABLES}
KEYS={"kh_customer_v1":b"fact-export-v1-public-test-key","kh_customer_v2":b"fact-export-v1-public-test-key-rotated"}
K1="kh_customer_v1"
K2="kh_customer_v2"
TIME=1762900000000000
TENANT="tn_acme_prod"
STREAM="station-facts-default"

def canonical(v):
    return json.dumps(v,sort_keys=True,separators=(",",":"),ensure_ascii=False)

def sha(data):
    return hashlib.sha256(data).hexdigest()

def lp(*parts):
    return b"".join(len(str(p).encode()).to_bytes(4,"big")+str(p).encode() for p in parts)

def pseudonym(domain,*parts,key=K1):
    return hmac.new(KEYS[key],lp("fact-export.v1/"+domain,*parts),hashlib.sha256).hexdigest()

PRODUCER=pseudonym("producer","fixture-producer")
SOURCE=pseudonym("source","fixture-source-a")
OTHER_SOURCE=pseudonym("source","fixture-source-b")
SCHEMA_DIGEST=sha("".join(f"{sha(canonical(SCHEMAS[t]).encode())}  tables/{t}.schema.json\n" for t in TABLES).encode())

def table_bytes(rows):
    return "".join(canonical(r)+"\n" for r in rows).encode()

def coverage_key(r):
    return sha(lp("fact-export.v1/coverage",r["source_id"],r["source_generation"],r["canonical_harness"],r["target_table"],r["coverage_dimension"]))

def blank(table,source=SOURCE,generation=1,key=K1,harness="codex"):
    row={}
    for name,rule in SCHEMAS[table]["properties"].items():
        typ=rule["type"]
        if isinstance(typ,list) and "null" in typ: value=None
        elif "enum" in rule: value=rule["enum"][0]
        elif typ=="string": value=pseudonym("fixture",table,name,key=key) if rule.get("pattern")=="^[0-9a-f]{64}$" else "fixture"
        elif typ=="integer": value=rule.get("minimum",0)
        elif typ=="boolean": value=False
        elif typ=="array": value=[]
        else: raise AssertionError((table,name))
        row[name]=value
    row.update(source_id=source,source_generation=generation,pseudonym_key_id=key,canonical_harness=harness)
    return row

def cov(table,rows,dimension,source,generation,key,harness):
    r=blank("coverage",source,generation,key,harness)
    count=len(rows)
    if dimension=="row_ingestion": observed=count
    elif dimension=="session_link_id": observed=sum(x["agent.session.link_id"]!="unlinked" for x in rows)
    elif dimension=="model_attribution": observed=sum(x["model_status"]!="unknown" for x in rows)
    elif dimension=="observed_cost_usd": observed=sum(x["cost_provenance"]!="unknown" for x in rows)
    elif dimension=="tool_outcome_pairing": observed=sum(x["outcome_status"] not in ("pending","unknown") for x in rows)
    elif dimension=="git_outcome_evidence":
        observed=sum(x["evidence_status"]!="missing" if table=="outcomes" else x["session_match_status"]=="matched" for x in rows)
    elif dimension=="timestamp_fidelity":
        field={"sessions":"timestamp_status","invocations":"occurred_at_status","tool_calls":"occurred_at_status","usage":"occurred_at_status","outcomes":"observed_at_status","git_evidence":"observed_at_status"}[table]
        observed=sum(x[field]==("complete" if table=="sessions" else "observed") for x in rows)
    else: raise AssertionError(dimension)
    r.update(target_table=table,coverage_dimension=dimension,discovery_status="discovered",read_status="complete",
        projection_status="projected_full",field_fidelity="exact",denominator_basis="raw_records_encountered",
        accepted_count=count,rejected_count=0,recognized_unprojected_count=0,encountered_count=count,
        expected_population_count=count,observable_count=observed,unknown_or_missing_count=count-observed,
        coverage_status="empty" if count==0 else "complete" if observed==count else "partial" if observed else "unavailable")
    reasons=SCHEMAS["coverage"]["properties"]["gap_reasons"]["items"]["enum"]
    r["gap_reasons"]=["none"] if r["coverage_status"] in ("complete","empty") else [next(x for x in reasons if x!="none")]
    r["coverage_key"]=coverage_key(r)
    return r

def recoverage(tables):
    tables["coverage"]=[]
    dimensions={"sessions":["session_link_id","timestamp_fidelity"],"invocations":["model_attribution","timestamp_fidelity"],
        "tool_calls":["tool_outcome_pairing","timestamp_fidelity"],"usage":["model_attribution","observed_cost_usd","timestamp_fidelity"],
        "outcomes":["git_outcome_evidence","timestamp_fidelity"],"git_evidence":["git_outcome_evidence","timestamp_fidelity"]}
    for table in TABLES[:-1]:
        groups={}
        for row in tables[table]:
            group=(row["source_id"],row["source_generation"],row["pseudonym_key_id"],row["canonical_harness"])
            groups.setdefault(group,[]).append(row)
        for (source,generation,key,harness),rows in sorted(groups.items()):
            for dimension in ["row_ingestion"]+dimensions[table]:
                tables["coverage"].append(cov(table,rows,dimension,source,generation,key,harness))
    return tables

def snapshot(labels=("codex-a",),generation=1,key=K1,source=SOURCE):
    result={t:[] for t in TABLES}
    for label in labels:
        harness=label.split("-")[0]
        session=pseudonym("session",source,harness,label,key=key)
        row=blank("sessions",source,generation,key,harness)
        row.update(session_key=session,org="acme:engineering",org_status="observed",endpoint_scope=pseudonym("endpoint_scope","fixture-endpoint",key=key),
            endpoint_scope_status="observed",team="platform",team_status="observed",link_epoch=None,unlinked_reason="invalid_key_material",
            origin="harness",harness_version="1.0.0",harness_version_status="observed",signal_source="session_log",
            session_status="completed",started_at_us=TIME,ended_at_us=TIME+1000,timestamp_status="complete",
            is_root=True,parent_session_key=None,lineage_relation="root",lineage_status="root")
        row["agent.session.link_id"]="unlinked"
        if harness=="unknown": row.update(origin="unknown",unlinked_reason="non_harness_origin")
        result["sessions"].append(row)
        message=pseudonym("message",session,"message-1",key=key)
        invocation=pseudonym("invocation",session,0,"call",message,key=key)
        row=blank("invocations",source,generation,key,harness)
        row.update(invocation_key=invocation,session_key=session,sequence_no=0,invocation_grain="call",message_key=message,
            call_id_missing_reason=None,model="fixture-model-v1",model_status="observed",model_unknown_reason=None,effort="medium",
            invocation_status="succeeded",occurred_at_us=TIME+10,occurred_at_status="observed",signal_source="session_log")
        result["invocations"].append(row)
        call=pseudonym("tool_call_id",session,"tool-1",key=key)
        row=blank("tool_calls",source,generation,key,harness)
        row.update(tool_call_key=pseudonym("tool_call",session,0,call,key=key),session_key=session,invocation_key=invocation,
            invocation_link_status="linked",sequence_no=0,call_id_hmac=call,call_id_status="observed",tool_name="Bash",
            tool_category="eng_build_test",tool_archetype="build_test_verify",structured_write=False,
            input_fingerprint_hmac=pseudonym("tool_input_fingerprint",canonical({"command":"fixture-build"}),key=key),
            fingerprint_status="observed",outcome_status="succeeded",outcome_unknown_reason=None,
            occurred_at_us=TIME+20,occurred_at_status="observed",duration_us=100,duration_status="observed",signal_source="session_log")
        result["tool_calls"].append(row)
        row=blank("usage",source,generation,key,harness)
        row.update(usage_key=pseudonym("usage",session,0,"per_turn",key=key),session_key=session,invocation_key=invocation,
            invocation_link_status="linked",sequence_no=0,occurred_at_us=TIME+100,occurred_at_status="observed",
            model="fixture-model-v1",model_status="observed",model_unknown_reason=None,effort="medium",
            usage_grain="per_turn",counter_semantics="per_turn_delta",predecessor_usage_key=None,
            input_uncached_tokens=20,output_tokens=10,cache_read_tokens=1,cache_write_tokens=2,total_tokens=33,
            reasoning_tokens=None,reasoning_status="not_reported",observed_usd_micros=14250,
            cost_provenance="harness_reported",cost_unknown_reason=None,signal_source="session_log")
        result["usage"].append(row)
        row=blank("outcomes",source,generation,key,harness)
        row.update(outcome_key=pseudonym("outcome",session,0,key=key),session_key=session,sequence_no=0,
            evidence_source="session_record_event",evidence_status="observed_full",missing_reason=None,
            repo_remote_hmac=pseudonym("repo_remote","https://example.invalid/org/repo",key=key),remote_status="observed",
            branch_name_hmac=pseudonym("branch","main",key=key),branch_status="named_branch",
            head_commit_hmac=pseudonym("commit","a"*40,key=key),head_commit_status="observed",
            base_commit_hmac=pseudonym("commit","b"*40,key=key),base_commit_status="observed",
            worktree_state="clean",observed_at_us=TIME+30,observed_at_status="observed")
        result["outcomes"].append(row)
    return recoverage(result)

def empty():
    return {t:[] for t in TABLES}

def event(sequence,rows=None,generation=1,key=K1,source=SOURCE,operation="upsert_replace",sessions=(),skip=None):
    rows=copy.deepcopy(rows if rows is not None else empty())
    manifest=dict(contract_id="fact-export-v1",contract_version=1,schema_digest=SCHEMA_DIGEST,
        producer=dict(producer_id=PRODUCER,producer_kind="dendro_cli",producer_version="0.5.0",pseudonym_key_id=key),
        tenant_placeholder=TENANT,stream=STREAM,snapshot_sequence=sequence,published_at_us=TIME+10000+sequence,
        replacement_scope=dict(scope_mode="session_keys" if sessions else "source_generation",operation=operation,
            reason="retention_or_redaction_purge" if operation=="purge_delete" else "initial_export",
            source_id=source,source_generation=generation,session_keys=list(sessions)),skip_reason=skip,files=[])
    if skip is None:
        for table in TABLES:
            data=table_bytes(rows[table])
            manifest["files"].append(dict(table=table,path=f"tables/{table}.jsonl",format="canonical_json_rows",
                row_count=len(rows[table]),byte_length=len(data),sha256=sha(data)))
    resign(manifest)
    return dict(committed_manifest=manifest,staged_tables=rows)

def resign(manifest):
    manifest["manifest_digest"]=sha(canonical({k:v for k,v in manifest.items() if k!="manifest_digest"}).encode())

class Scenario:
    """Author-declared expected transitions only; contains no acceptance logic."""
    def __init__(self,name,description):
        self.name=name+".json"
        self.description=description
        self.tables={t:{} for t in TABLES}
        self.active={}; self.purged={}; self.tombstones=set(); self.keys={}; self.consumed={}; self.cursor=0; self.coverage=[]; self.steps=[]
        self.rounding=[]
    def add(self,e,status="APPLIED",consume=True,label=None):
        e=copy.deepcopy(e)
        m=e.get("committed_manifest")
        if status=="APPLIED":
            scope=m["replacement_scope"]; source=scope["source_id"]; generation=scope["source_generation"]
            if scope["scope_mode"]=="session_keys":
                chosen=set(scope["session_keys"])
                for t in TABLES[:-1]:
                    self.tables[t]={k:r for k,r in self.tables[t].items() if not (r["source_id"]==source and r.get("session_key") in chosen)}
                self.tombstones.update(chosen)
            else:
                for t in TABLES:
                    self.tables[t]={k:r for k,r in self.tables[t].items() if r["source_id"]!=source}
                    if scope["operation"]=="upsert_replace":
                        self.tables[t].update({r[PK[t]]:copy.deepcopy(r) for r in e["staged_tables"][t]})
                self.active[source]=generation
                if scope["operation"]=="purge_delete": self.purged[source]=generation
                else: self.keys[source]=m["producer"]["pseudonym_key_id"]
        if consume:
            self.cursor=m["snapshot_sequence"]
            declared=m.get("manifest_digest")
            raw=e.get("raw_manifest_utf8",canonical(m))
            digest=declared if isinstance(declared,str) and len(declared)==64 and all(x in "0123456789abcdef" for x in declared) else sha(raw.encode("utf-8"))
            self.consumed[self.cursor]=digest
            kind="accepted" if status=="APPLIED" else "skipped" if status=="SKIPPED" else "rejected"
            self.coverage.append(dict(tenant_placeholder=TENANT,producer_id=PRODUCER,stream=STREAM,snapshot_sequence=self.cursor,
                manifest_digest=digest,status=kind,reason=None if kind=="accepted" else m["skip_reason"] if kind=="skipped" else status))
        ordered={t:[self.tables[t][k] for k in sorted(self.tables[t])] for t in TABLES}
        e.update(step_id=label or f"step-{len(self.steps)+1}",expected_step_result=dict(status=status,high_water_sequence=self.cursor,
            active_row_counts={t:len(self.tables[t]) for t in TABLES},import_coverage=copy.deepcopy(self.coverage),
            active_table_digests={t:sha(table_bytes(ordered[t])) for t in TABLES}))
        self.steps.append(e)
        return self
    def save(self):
        final=dict(high_water_sequence=self.cursor,committed_sequences=sorted(self.consumed),active_source_generations=self.active,
            purged_source_generations=self.purged,tombstoned_session_keys=sorted(self.tombstones),source_key_ids=self.keys,
            active_tables={t:[self.tables[t][k] for k in sorted(self.tables[t])] for t in TABLES})
        doc=dict(contract_id="fact-export-v1",contract_version=1,vector_file=self.name,description=self.description,
            expected_schema_digest=SCHEMA_DIGEST,expected_tenant_placeholder=TENANT,expected_producer_id=PRODUCER,
            expected_stream=STREAM,steps=self.steps,expected_final_state=final)
        if self.rounding: doc["rounding_cases"]=self.rounding
        (BASE/"vectors"/self.name).write_text(json.dumps(doc,sort_keys=True,indent=2,ensure_ascii=False)+"\n",encoding="utf-8",newline="\n")

def aggregate(row,ordinal=0):
    r=copy.deepcopy(row)
    r.update(usage_grain="session_aggregate",counter_semantics="session_aggregate",invocation_key=None,invocation_link_status="session_level_summary",sequence_no=ordinal)
    r["usage_key"]=pseudonym("usage",r["session_key"],ordinal,"session_aggregate",key=r["pseudonym_key_id"])
    return r

def main():
    scenarios=[]
    def case(name,description):
        result=Scenario(name,description); scenarios.append(result); return result
    good=snapshot()
    s=case("clean_sequence","Two independent sources, then a full source generation advance.")
    s.add(event(1,good)).add(event(2,snapshot(("claude-b",),source=OTHER_SOURCE),source=OTHER_SOURCE)).add(event(3,snapshot(generation=2),generation=2))

    pending=snapshot(("codex-a","codex-b"))
    pending["tool_calls"][0].update(outcome_status="pending",outcome_unknown_reason="awaiting_tool_result",duration_us=None,duration_status="single_point_only")
    pending["invocations"][0].update(model=None,model_status="unknown",model_unknown_reason="pending_turn_attribution")
    recoverage(pending)
    both=snapshot(("codex-a","codex-b")); removed=both["sessions"][0]["session_key"]
    s=case("outcome_replacement","Full source re-export updates pending outcome/model; targeted purge preserves survivors and coverage bytes.")
    s.add(event(1,pending)).add(event(2,both)).add(event(3,operation="purge_delete",sessions=[removed]))
    s.add(event(4,operation="purge_delete",sessions=[removed],generation=2),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    s.add(event(5,snapshot(("codex-b",))))

    s=case("stale_out_of_order","A true gap waits; a consumed stale-generation rejection releases the next sequence.")
    s.add(event(1,snapshot(generation=2),generation=2))
    next_event=event(3,snapshot(generation=3),generation=3)
    s.add(next_event,"GAP_PENDING",False).add(event(2,good),"REJECTED_STALE_GENERATION_OR_TOMBSTONE").add(next_event)

    s=case("duplicate_idempotent","Same digest on any old consumed sequence is idempotent; any different digest conflicts.")
    first=event(1,good); second=event(2,snapshot(generation=2),generation=2)
    s.add(first).add(second).add(first,"DUPLICATE_IDEMPOTENT",False)
    conflict=copy.deepcopy(first); conflict["committed_manifest"]["published_at_us"]+=1; resign(conflict["committed_manifest"])
    s.add(conflict,"CONFLICTING_DUPLICATE",False)
    changed_digest=copy.deepcopy(first); changed_digest["committed_manifest"]["manifest_digest"]="f"*64
    s.add(changed_digest,"CONFLICTING_DUPLICATE",False,label="declared-digest-differs-before-equality-check")
    bad=copy.deepcopy(good); bad["tool_calls"][0]["tool_name"]="PrivateTool"
    rejected=event(3,bad)
    s.add(rejected,"REJECTED_ROW_SCHEMA_VIOLATION").add(event(4,snapshot(generation=2),generation=2))
    s.add(rejected,"DUPLICATE_IDEMPOTENT",False).add(conflict,"CONFLICTING_DUPLICATE",False)

    s=case("partial_publication","Absent manifest is invisible. Next sequence waits, producer skips unused slot, next sequence imports. No visible slot is republished.")
    s.add(dict(committed_manifest=None,staged_tables=good),"IGNORED_UNCOMMITTED_NO_MANIFEST",False)
    next_event=event(2,good)
    s.add(next_event,"GAP_PENDING",False).add(event(1,skip="abandoned_publication"),"SKIPPED").add(next_event)

    s=case("schema_mismatch","Schema mismatch consumes a rejected slot and does not prevent the next sequence.")
    bad=event(1,good); bad["committed_manifest"]["schema_digest"]="0"*64; resign(bad["committed_manifest"])
    s.add(bad,"REJECTED_SCHEMA_MISMATCH").add(event(2,good)).add(bad,"DUPLICATE_IDEMPOTENT",False)

    unknown=snapshot()
    unknown["sessions"][0].update(started_at_us=None,ended_at_us=None,timestamp_status="not_observable")
    unknown["invocations"][0].update(model=None,model_status="unknown",model_unknown_reason="not_reported_in_event",occurred_at_us=None,occurred_at_status="not_observable")
    unknown["tool_calls"][0].update(outcome_status="pending",outcome_unknown_reason="awaiting_tool_result",duration_us=None,duration_status="single_point_only")
    unknown["usage"][0].update(model=None,model_status="unknown",model_unknown_reason="not_reported_by_harness",observed_usd_micros=None,cost_provenance="unknown",cost_unknown_reason="subscription_unmetered")
    outcome=unknown["outcomes"][0]
    for field,status in [("repo_remote_hmac","remote_status"),("branch_name_hmac","branch_status"),("head_commit_hmac","head_commit_status"),("base_commit_hmac","base_commit_status")]:
        outcome[field]=None; outcome[status]="not_observed"
    reason=next(x for x in SCHEMAS["outcomes"]["properties"]["missing_reason"]["enum"] if x is not None)
    outcome.update(evidence_status="missing",missing_reason=reason,observed_at_us=None,observed_at_status="not_observable")
    recoverage(unknown)
    pi=cov("sessions",[],"row_ingestion",SOURCE,1,K1,"pi")
    pi.update(projection_status="connector_unimplemented",recognized_unprojected_count=4,encountered_count=4,expected_population_count=4,unknown_or_missing_count=4,coverage_status="unavailable")
    pi["gap_reasons"]=[next(x for x in SCHEMAS["coverage"]["properties"]["gap_reasons"]["items"]["enum"] if x!="none")]
    unknown["coverage"].append(pi)
    case("coverage_missingness","Unknown fields remain null; missing connector has explicit denominators, never phantom rows.").add(event(1,unknown)).add(event(2,good))

    s=case("gap_recovery","Sequence 3 waits for 1 and 2; arrivals recover without consuming the pending sequence.")
    third=event(3,snapshot(generation=3),generation=3)
    s.add(third,"GAP_PENDING",False).add(event(1,good)).add(third,"GAP_PENDING",False)
    s.add(event(2,snapshot(generation=2),generation=2)).add(third)

    s=case("gap_explicit_skip","Only a producer-issued explicit skip fills a true gap.")
    second=event(2,good)
    s.add(second,"GAP_PENDING",False).add(event(1,skip="producer_recovery"),"SKIPPED").add(second)
    s.add(event(3,skip="source_unavailable"),"SKIPPED").add(event(4,good))

    s=case("resurrection","Permanent session purge survives higher generations and a source-generation purge.")
    s.add(event(1,both)).add(event(2,operation="purge_delete",sessions=[removed]))
    s.add(event(3,snapshot(("codex-a","codex-b"),generation=2),generation=2),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    s.add(event(4,snapshot(("codex-b",),generation=2),generation=2))
    s.add(event(5,snapshot(("codex-a",),generation=99),generation=99),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    s.add(event(6,generation=2,operation="purge_delete"))
    s.add(event(7,snapshot(("codex-a",),generation=3),generation=3),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")

    many=snapshot(("claude-a","codex-a","unknown-a")); unknown_key=many["sessions"][-1]["session_key"]
    case("multi_harness","A single source-generation scope covers multiple harnesses including unknown.").add(event(1,many)).add(event(2,operation="purge_delete",sessions=[unknown_key])).add(event(3,snapshot(("claude-a","codex-a"),generation=2),generation=2))

    s=case("rejection_recovery","Digest, file, row and schema rejections are consumed; later sequences proceed normally.")
    bad=event(1,good); bad["committed_manifest"]["manifest_digest"]="0"*64
    s.add(bad,"REJECTED_INVALID_MANIFEST")
    bad=event(2,good); bad["staged_tables"].pop("usage")
    s.add(bad,"REJECTED_FILE_INTEGRITY")
    bad=copy.deepcopy(good); bad["usage"][0]["observed_usd"]="0.014250"
    s.add(event(3,bad),"REJECTED_ROW_SCHEMA_VIOLATION").add(event(4,good))
    bad=event(5,good); bad["committed_manifest"]["schema_digest"]="1"*64; resign(bad["committed_manifest"])
    s.add(bad,"REJECTED_SCHEMA_MISMATCH").add(event(6,good))

    s=case("key_rotation","Key change requires purging ALL old-key source rows then full re-export at a higher generation. Informative limitation: tombstones use the key in force; after rotation the importer cannot detect re-insertion of a purged native session under a new pseudonym. The producer MUST exclude it using a native-identity purge ledger, verified by EN5c; no stable cross-rotation session identifier is exported because it would defeat unlinkability on every session row.")
    rotated=snapshot(generation=2,key=K2)
    s.add(event(1,good)).add(event(2,rotated,generation=2,key=K2),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    s.add(event(3,operation="purge_delete")).add(event(4,good),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    s.add(event(5,rotated,generation=2,key=K2))
    s.add(event(6,snapshot(generation=3),generation=3),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")

    merge=snapshot()
    for index,status in enumerate(("matched","unmatched","ambiguous","unknown")):
        row=blank("git_evidence",SOURCE,1,K1,"codex" if status=="matched" else "unknown")
        commit=("a"*40 if index==0 else format(index+10,"x")*40)
        row.update(commit_hmac=pseudonym("commit",commit),parent_hmacs=[pseudonym("commit","b"*40),pseudonym("commit","c"*40)] if index==0 else [],
            session_key=merge["sessions"][0]["session_key"] if status=="matched" else None,
            session_match_status=status,is_merge=index==0,merge_status="observed",
            on_default_branch=True if index==0 else None,default_branch_status="observed" if index==0 else "unknown",
            observed_at_us=TIME-index,observed_at_status="observed",evidence_source="local_git_history")
        row["git_evidence_key"]=pseudonym("git_evidence",SOURCE,row["commit_hmac"],row["observed_at_us"],row["session_key"] or "",status)
        merge["git_evidence"].append(row)
    # SHA-256 repository canonical full commit input.
    row=copy.deepcopy(merge["git_evidence"][1]); row["commit_hmac"]=pseudonym("commit","d"*64)
    row["git_evidence_key"]=pseudonym("git_evidence",SOURCE,row["commit_hmac"],row["observed_at_us"],"","unmatched")
    merge["git_evidence"].append(row)
    recoverage(merge)
    s=case("merge_evidence","LOCAL historical merge/default-branch evidence, keyed ordered parents, matched/unmatched/ambiguous/unknown and SHA-256 repository.")
    s.add(event(1,merge))
    bad=copy.deepcopy(merge); bad["git_evidence"][0]["commit_hmac"]="a"*40
    s.add(event(2,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    bad=copy.deepcopy(merge); bad["git_evidence"][0]["parent_hmacs"]=["b"*40]
    s.add(event(3,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    bad=copy.deepcopy(merge); bad["git_evidence"][0]["session_match_status"]="ambiguous"
    s.add(event(4,bad),"REJECTED_ROW_SCHEMA_VIOLATION")

    s=case("tool_name_mcp","Only closed built-ins or full keyed mcp_hmac names; no arbitrary slug or truncated digest.")
    custom=copy.deepcopy(good); custom["tool_calls"][0]["tool_name"]="mcp_hmac_"+pseudonym("tool_name","mcp-private-fixture")
    s.add(event(1,custom))
    for q,name in enumerate(("PrivateTool","mcp_hmac_"+"a"*16,"mcp_private_server_operation"),2):
        bad=copy.deepcopy(good); bad["tool_calls"][0]["tool_name"]=name
        s.add(event(q,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    s.add(event(5,good))

    s=case("coverage_completeness","Exactly one deterministic row_ingestion coverage tuple for each populated fact partition.")
    bad=copy.deepcopy(good); bad["coverage"]=[]
    s.add(event(1,bad),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
    bad=copy.deepcopy(good); bad["coverage"]=[r for r in bad["coverage"] if r["coverage_dimension"]!="row_ingestion"]
    s.add(event(2,bad),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH").add(event(3,good))
    bad=copy.deepcopy(good); bad["coverage"][0]["coverage_key"]="f"*64
    s.add(event(4,bad),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
    bad=copy.deepcopy(good); duplicate=copy.deepcopy(bad["coverage"][0]); duplicate["coverage_key"]="e"*64; bad["coverage"].append(duplicate)
    s.add(event(5,bad),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")

    s=case("usage_grain","Per-session aggregate/per-turn exclusivity, one aggregate maximum and declared grain consistency.")
    s.add(event(1,good))
    bad=copy.deepcopy(good); bad["usage"].append(aggregate(bad["usage"][0],1)); recoverage(bad)
    s.add(event(2,bad),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
    bad=copy.deepcopy(good); bad["usage"]=[aggregate(bad["usage"][0],0),aggregate(bad["usage"][0],1)]; recoverage(bad)
    s.add(event(3,bad),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
    single=copy.deepcopy(good); single["usage"]=[aggregate(single["usage"][0])]; recoverage(single)
    s.add(event(4,single))
    bad=copy.deepcopy(good); bad["usage"][0]["usage_grain"]="session_aggregate"
    s.add(event(5,bad),"REJECTED_ROW_SCHEMA_VIOLATION")

    s=case("attribution_status","C1a 1024-byte ASCII slug grammar, pseudonymized endpoint, explicit null reason statuses.")
    long=copy.deepcopy(good); long["sessions"][0]["org"]="A"*1024; long["sessions"][0]["team"]=None; long["sessions"][0]["team_status"]="missing_team"
    s.add(event(1,long))
    missing=copy.deepcopy(good); missing["sessions"][0].update(org=None,org_status="missing_org",unlinked_reason="missing_org")
    s.add(event(2,recoverage(missing)))
    invalid=copy.deepcopy(good); invalid["sessions"][0].update(endpoint_scope=None,endpoint_scope_status="invalid_field_characters",unlinked_reason="invalid_field_characters")
    s.add(event(3,recoverage(invalid)))
    for q,value in enumerate(("has space","nonascii-\u00e9","a"*1025),4):
        bad=copy.deepcopy(good); bad["sessions"][0]["org"]=value
        s.add(event(q,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    bad=copy.deepcopy(long); bad["sessions"][0]["team_status"]="missing_org"
    s.add(event(7,bad),"REJECTED_ROW_SCHEMA_VIOLATION")

    s=case("tenant_stream_auth","Tenant, producer and stream are authenticated before any state mutation.")
    s.add(event(1,good))
    for field,value in (("tenant_placeholder","tn_other_tenant"),("stream","other-stream"),("producer_id","f"*64)):
        bad=event(2,good)
        if field=="producer_id": bad["committed_manifest"]["producer"][field]=value
        else: bad["committed_manifest"][field]=value
        resign(bad["committed_manifest"]); s.add(bad,"REJECTED_TENANT_OR_PRODUCER_MISMATCH",False)
    s.add(event(2,good))

    s=case("cost_rounding","Only observed INT64 cost deltas. Exact binary64-to-micros ties round to even.")
    s.add(event(1,good))
    for value,result in [(0.0,0),(0.0078125,7812),(0.0234375,23438),(0.015625,15625),(1.0,1000000),(-1.0,None),(float("inf"),None),(1e20,None),(5e-324,0)]:
        s.rounding.append(dict(binary64_hex=struct.pack(">d",value).hex(),expected_micros=result))

    s=case("tombstone_cross_source","A session tombstone is scoped to tenant/producer/stream, not to its original source.")
    s.add(event(1,good)); tomb=good["sessions"][0]["session_key"]
    s.add(event(2,operation="purge_delete",sessions=[tomb]))
    bad=copy.deepcopy(good)
    for table in TABLES:
        for row in bad[table]: row["source_id"]=OTHER_SOURCE
    recoverage(bad)
    s.add(event(3,bad,source=OTHER_SOURCE),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")

    s=case("purge_guards","Targeted purge requires active generation and empty payloads; source purge may use active generation.")
    s.add(event(1,both))
    s.add(event(2,generation=2,operation="purge_delete",sessions=[removed]),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    bad=empty(); bad["coverage"]=copy.deepcopy(both["coverage"])
    s.add(event(3,bad,operation="purge_delete",sessions=[removed]),"REJECTED_REFERENTIAL_OR_COVERAGE_MISMATCH")
    s.add(event(4,operation="purge_delete",sessions=[removed])).add(event(5,operation="purge_delete"))
    s.add(event(6,snapshot(("codex-b",),generation=2),generation=2))

    s=case("purge_after_generation_purge","A targeted tombstone at the active generation remains valid after generation purge, even with no active rows.")
    s.add(event(1,both)).add(event(2,operation="purge_delete"))
    s.add(event(3,operation="purge_delete",sessions=[removed]))
    s.add(event(4,snapshot(("codex-a",),generation=2),generation=2),"REJECTED_STALE_GENERATION_OR_TOMBSTONE")
    s.add(event(5,snapshot(("codex-b",),generation=2),generation=2))

    s=case("row_constraints","Shared INT64, enum, model length, timestamp and zero-content rules.")
    s.add(event(1,good))
    mutations=[
        ("usage","input_uncached_tokens",2**63),("usage","input_uncached_tokens",True),
        ("usage","observed_usd_micros",1.5),("invocations","model","m"*129),
        ("invocations","occurred_at_us",0),("invocations","effort","ultra"),
        ("invocations","model","sk-private"),("tool_calls","tool_name",None)]
    for q,(table,field,value) in enumerate(mutations,2):
        bad=copy.deepcopy(good); bad[table][0][field]=value
        s.add(event(q,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    xhigh=copy.deepcopy(good)
    for table in ("invocations","usage"): xhigh[table][0]["effort"]="xhigh"
    s.add(event(10,xhigh))

    linked=copy.deepcopy(good)
    # Public pre-derived 32-byte C1a fixture key, never an organization secret.
    linked["sessions"][0]["agent.session.link_id"]=hmac.new(bytes(range(32)),
        lp("agent.session.link/v1","acme:engineering","fixture-endpoint","codex","codex-a"),hashlib.sha256).hexdigest()
    linked["sessions"][0].update(link_epoch="fixture-epoch-v1",unlinked_reason=None)
    recoverage(linked)
    s=case("linked_sessions","Valid C1a-linked row with pseudonymized endpoint; linked status requires reason/epoch consistency.")
    s.add(event(1,linked))
    bad=copy.deepcopy(linked); bad["sessions"][0]["link_epoch"]=None
    s.add(event(2,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    bad=copy.deepcopy(linked); bad["sessions"][0]["origin"]="synthetic"
    s.add(event(3,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    bad=copy.deepcopy(good); bad["sessions"][0]["endpoint_scope"]="raw-endpoint-slug"
    s.add(event(4,bad),"REJECTED_ROW_SCHEMA_VIOLATION")

    s=case("removed_scope_modes","Removed full_stream is a structural manifest error and cannot bypass session tombstones.")
    s.add(event(1,good))
    bad=event(2,good); bad["committed_manifest"]["replacement_scope"]["scope_mode"]="full_stream"; resign(bad["committed_manifest"])
    s.add(bad,"REJECTED_INVALID_MANIFEST").add(event(3,good))

    s=case("removed_supersedes","supersedes_generations is not a v1 manifest field.")
    s.add(event(1,good))
    bad=event(2,good); bad["committed_manifest"]["replacement_scope"]["supersedes_generations"]=[]; resign(bad["committed_manifest"])
    s.add(bad,"REJECTED_INVALID_MANIFEST").add(event(3,good))

    s=case("invalid_manifest_shape","Shape failures consume the next slot, preserve facts and record declared digest or exact raw-byte SHA256. Gaps and duplicates take priority.")
    s.add(event(1,good))
    pending=event(3,good); pending["committed_manifest"].pop("files"); resign(pending["committed_manifest"])
    s.add(pending,"GAP_PENDING",False)
    invalid=event(2,good); invalid["committed_manifest"]["published_at_us"]=0; resign(invalid["committed_manifest"])
    s.add(invalid,"REJECTED_INVALID_MANIFEST").add(invalid,"DUPLICATE_IDEMPOTENT",False)
    conflict=copy.deepcopy(invalid); conflict["committed_manifest"]["manifest_digest"]="a"*64
    s.add(conflict,"CONFLICTING_DUPLICATE",False)
    s.add(pending,"REJECTED_INVALID_MANIFEST").add(event(4,good))
    invalid=event(5,good); invalid["committed_manifest"]["manifest_digest"]="INVALID"
    invalid["raw_manifest_utf8"]=" \n"+json.dumps(invalid["committed_manifest"],indent=1)+"\n "
    s.add(invalid,"REJECTED_INVALID_MANIFEST").add(event(6,good))
    s.add(invalid,"CONFLICTING_DUPLICATE",False)
    invalid=event(7,good); invalid["committed_manifest"].pop("manifest_digest")
    s.add(invalid,"REJECTED_INVALID_MANIFEST").add(event(8,good))
    s.add(invalid,"CONFLICTING_DUPLICATE",False)
    invalid=event(9,good); invalid["committed_manifest"]["manifest_digest"]=42
    s.add(invalid,"REJECTED_INVALID_MANIFEST").add(event(10,good))

    s=case("unparseable_manifest_identity","Unparseable stream keys and non-positive/non-INT64 sequences are ignored without coverage or consumption.")
    for field,value in (("snapshot_sequence",0),("snapshot_sequence",True),("snapshot_sequence",2**63),("snapshot_sequence","1"),("tenant_placeholder",None),("stream","INVALID"),("producer",{})):
        invalid=event(1,good); invalid["committed_manifest"][field]=value
        s.add(invalid,"IGNORED_UNPARSEABLE_MANIFEST_KEY_OR_SEQUENCE",False)
    invalid=event(1,good); invalid["raw_manifest_utf8"]="{broken JSON"
    s.add(invalid,"IGNORED_UNPARSEABLE_MANIFEST_KEY_OR_SEQUENCE",False)
    s.add(event(1,good))

    s=case("model_vocabulary","Invocation and usage share observed/inferred/unknown statuses and all six unknown reasons.")
    inferred=copy.deepcopy(good)
    for t in ("invocations","usage"): inferred[t][0]["model_status"]="inferred_from_session"
    s.add(event(1,recoverage(inferred)))
    for q,reason in enumerate(("not_reported_in_event","not_reported_by_harness","pending_turn_attribution","awaiting_turn_completion","redacted_unallowlisted","redacted_unrecognized_format"),2):
        unknown=copy.deepcopy(good)
        for t in ("invocations","usage"): unknown[t][0].update(model=None,model_status="unknown",model_unknown_reason=reason)
        s.add(event(q,recoverage(unknown)))
    for q,t in enumerate(("invocations","usage"),8):
        bad=copy.deepcopy(inferred); bad[t][0]["model_unknown_reason"]="pending_turn_attribution"
        s.add(event(q,bad),"REJECTED_ROW_SCHEMA_VIOLATION")
    s.add(event(10,good))

    for s in scenarios: s.save()
    vector_paths=sorted((BASE/"vectors").glob("*.json"))
    (BASE/"vectors/SHA256SUMS").write_text("".join(f"{sha(p.read_bytes())}  {p.name}\n" for p in vector_paths),encoding="utf-8",newline="\n")
    paths=sorted(["SNAPSHOT.md","tables/README.md","vectors/SHA256SUMS"]+[f"tables/{t}.schema.json" for t in TABLES]+[p.relative_to(BASE).as_posix() for p in vector_paths])
    sums="".join(f"{sha((BASE/p).read_bytes())}  {p}\n" for p in paths).encode()
    (BASE/"SHA256SUMS").write_bytes(sums)
    (BASE/"CONTRACT-DIGEST").write_text(sha(sums)+"\n",encoding="utf-8",newline="\n")
    print(f"Generated {len(scenarios)} vectors, {sum(len(s.steps) for s in scenarios)} steps.")
    print("schema_digest="+SCHEMA_DIGEST)
    print("CONTRACT-DIGEST="+sha(sums))

if __name__=="__main__":
    main()
