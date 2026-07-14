#!/usr/bin/env python3
"""Generate deterministic Phase 0 inventory artifacts for the Apollo source corpus.

This tool intentionally performs only lexical repository forensics.  It does not
assemble source, resolve symbols, or infer behavioral dependencies.
"""

from __future__ import annotations

import argparse
import csv
import difflib
import hashlib
import io
import json
import re
import subprocess
from pathlib import Path


PROGRAMS = {
    "Comanche055": "Command Module",
    "Luminary099": "Lunar Module",
}

SUBSYSTEM_RULES = {
    "source-and-memory-layout": (
        "ASSEMBLY",
        "TAGS_FOR_RELATIVE_SETLOC",
        "CONTROLLED_CONSTANTS",
        "FLAGWORD",
        "ERASABLE_ASSIGNMENTS",
        "FIXED_FIXED_CONSTANT",
        "INPUT_OUTPUT_CHANNEL",
    ),
    "interrupts-and-timing": (
        "INTERRUPT",
        "T4RUPT",
        "T6-RUPT",
        "KEYRUPT",
        "UPRUPT",
    ),
    "executive-waitlist-restart": (
        "EXECUTIVE",
        "WAITLIST",
        "PHASE_TABLE",
        "FRESH_START",
        "RESTART",
        "ALARM_AND_ABORT",
        "SERVICE_ROUTINES",
        "INTER-BANK_COMMUNICATION",
        "SELF-CHECK",
        "SELF_CHECK",
    ),
    "interpreter-and-numerics": (
        "INTERPRETER",
        "INTERPRETIVE",
        "SINGLE_PRECISION",
        "CONIC",
        "INTEGRATION",
        "MEASUREMENT",
        "TIME_OF_FREE_FALL",
        "LATITUDE_LONGITUDE",
    ),
    "dsky-and-crew-interface": (
        "PINBALL",
        "DISPLAY",
        "EXTENDED_VERBS",
    ),
    "guidance-and-navigation": (
        "GUIDANCE",
        "LUNAR_LANDING",
        "LAMBERT",
        "EPHEMERIDES",
        "STABLE_ORBIT",
        "GEOMETRY",
        "ALIGNMENT",
        "ATTITUDE_MANEUVER",
        "KALMAN_FILTER",
    ),
    "vehicle-control": (
        "DAP",
        "AUTOPILOT",
        "JET_SELECTION",
        "THROTTLE",
        "TVC",
        "GIMBAL",
        "STEERING",
        "TRIM",
    ),
    "communications-sensors-and-telemetry": (
        "DOWNLINK",
        "TELEMETRY",
        "S-BAND",
        "RADAR",
        "AOTMARK",
        "SXTMARK",
        "IMU_",
    ),
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def line_count(path: Path) -> int:
    with path.open("rb") as source:
        return sum(1 for _ in source)


def git_value(source: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(source), *args], text=True
    ).strip()


def parse_main(main: Path) -> list[dict[str, object]]:
    includes: list[dict[str, object]] = []
    for line_number, line in enumerate(main.read_text(encoding="utf-8").splitlines(), 1):
        if not line.startswith("$"):
            continue
        token = line[1:].split()[0]
        page_match = re.search(r"#\s*(.+)$", line)
        includes.append(
            {
                "order": len(includes) + 1,
                "line": line_number,
                "token": token,
                "listing_pages": page_match.group(1).strip() if page_match else None,
            }
        )
    return includes


def subsystem_candidates(filename: str) -> list[str]:
    upper = filename.upper()
    candidates = [
        subsystem
        for subsystem, needles in SUBSYSTEM_RULES.items()
        if any(needle in upper for needle in needles)
    ]
    if re.match(r"^[PR][0-9]", upper):
        candidates.append("mission-programs-and-routines")
    return sorted(set(candidates)) or ["unclassified"]


def close_candidates(token: str, filenames: list[str]) -> list[str]:
    return difflib.get_close_matches(token, filenames, n=3, cutoff=0.72)


def build_inventory(source: Path) -> tuple[dict[str, object], dict[str, object], str, str]:
    commit = git_value(source, "rev-parse", "HEAD")
    branch = git_value(source, "rev-parse", "--abbrev-ref", "HEAD")
    remote = git_value(source, "remote", "get-url", "origin")
    dirty = bool(git_value(source, "status", "--porcelain"))

    programs: list[dict[str, object]] = []
    graph_nodes: list[dict[str, object]] = []
    graph_edges: list[dict[str, object]] = []
    all_modules: dict[str, dict[str, dict[str, object]]] = {}
    manifest_lines: list[str] = []

    for program, vehicle in PROGRAMS.items():
        program_dir = source / program
        files = sorted(program_dir.glob("*.agc"), key=lambda path: path.name)
        filenames = [path.name for path in files]
        includes = parse_main(program_dir / "MAIN.agc")
        include_by_token = {str(item["token"]): item for item in includes}

        modules: list[dict[str, object]] = []
        program_modules: dict[str, dict[str, object]] = {}
        for path in files:
            relative = path.relative_to(source).as_posix()
            digest = sha256(path)
            include = include_by_token.get(path.name)
            module = {
                "path": relative,
                "filename": path.name,
                "sha256": digest,
                "lines": line_count(path),
                "included_from_main_exactly": include is not None,
                "include_order": include["order"] if include else None,
                "candidate_subsystems": subsystem_candidates(path.name),
            }
            modules.append(module)
            program_modules[path.name] = module
            manifest_lines.append(f"{digest}  {relative}")
            graph_nodes.append(
                {
                    "id": relative,
                    "kind": "main" if path.name == "MAIN.agc" else "source-module",
                    "program": program,
                }
            )

        unresolved: list[dict[str, object]] = []
        for include in includes:
            token = str(include["token"])
            target = f"{program}/{token}"
            if token in program_modules:
                status = "exact"
            else:
                status = "unresolved"
                target = f"{program}/__unresolved__/{token}"
                candidates = close_candidates(token, filenames)
                unresolved.append(
                    {
                        **include,
                        "candidate_matches": candidates,
                        "resolution_status": "unresolved",
                    }
                )
                graph_nodes.append(
                    {
                        "id": target,
                        "kind": "unresolved-include",
                        "program": program,
                        "candidate_matches": candidates,
                    }
                )
            graph_edges.append(
                {
                    "source": f"{program}/MAIN.agc",
                    "target": target,
                    "kind": "textual-include",
                    "order": include["order"],
                    "status": status,
                    "source_line": include["line"],
                }
            )

        total_lines = sum(int(module["lines"]) for module in modules)
        programs.append(
            {
                "id": program,
                "vehicle": vehicle,
                "main": f"{program}/MAIN.agc",
                "file_count": len(modules),
                "line_count": total_lines,
                "include_count": len(includes),
                "unresolved_include_count": len(unresolved),
                "unresolved_includes": unresolved,
                "modules": modules,
            }
        )
        all_modules[program] = program_modules

    comanche_names = set(all_modules["Comanche055"])
    luminary_names = set(all_modules["Luminary099"])
    shared_names = sorted(comanche_names & luminary_names)
    shared_modules = []
    for filename in shared_names:
        comanche = all_modules["Comanche055"][filename]
        luminary = all_modules["Luminary099"][filename]
        shared_modules.append(
            {
                "filename": filename,
                "byte_identical": comanche["sha256"] == luminary["sha256"],
                "comanche_path": comanche["path"],
                "luminary_path": luminary["path"],
            }
        )

    inventory = {
        "schema_version": 1,
        "scope": "lexical Phase 0 repository inventory; not a semantic call graph",
        "source": {
            "repository": remote,
            "commit": commit,
            "branch": branch,
            "dirty": dirty,
        },
        "totals": {
            "programs": len(programs),
            "agc_files": sum(int(program["file_count"]) for program in programs),
            "agc_lines": sum(int(program["line_count"]) for program in programs),
            "same_filename_modules": len(shared_modules),
            "byte_identical_same_filename_modules": sum(
                1 for module in shared_modules if module["byte_identical"]
            ),
        },
        "programs": programs,
        "cross_program": {
            "same_filename_modules": shared_modules,
            "comanche_only_filenames": sorted(comanche_names - luminary_names),
            "luminary_only_filenames": sorted(luminary_names - comanche_names),
        },
    }
    graph = {
        "schema_version": 1,
        "scope": "MAIN.agc textual include graph only",
        "source_commit": commit,
        "nodes": sorted(graph_nodes, key=lambda node: str(node["id"])),
        "edges": sorted(
            graph_edges,
            key=lambda edge: (str(edge["source"]), int(edge["order"])),
        ),
    }

    csv_buffer = io.StringIO()
    fieldnames = [
        "program",
        "vehicle",
        "path",
        "filename",
        "lines",
        "sha256",
        "included_from_main_exactly",
        "include_order",
        "candidate_subsystems",
    ]
    writer = csv.DictWriter(csv_buffer, fieldnames=fieldnames, lineterminator="\n")
    writer.writeheader()
    for program in programs:
        for module in program["modules"]:
            writer.writerow(
                {
                    "program": program["id"],
                    "vehicle": program["vehicle"],
                    **{key: module[key] for key in fieldnames if key in module},
                    "candidate_subsystems": ";".join(module["candidate_subsystems"]),
                }
            )

    manifest = "\n".join(sorted(manifest_lines)) + "\n"
    return inventory, graph, csv_buffer.getvalue(), manifest


def dot_graph(graph: dict[str, object]) -> str:
    lines = [
        "digraph apollo11_includes {",
        '  graph [rankdir="LR", label="Apollo 11 MAIN.agc textual includes"];',
        '  node [shape="box", fontname="monospace", fontsize="9"];',
    ]
    for node in graph["nodes"]:
        node_id = json.dumps(node["id"])
        label = json.dumps(str(node["id"]).split("/")[-1])
        if node["kind"] == "main":
            attrs = f"label={label}, shape=folder, style=filled, fillcolor=lightblue"
        elif node["kind"] == "unresolved-include":
            attrs = f"label={label}, style=filled, fillcolor=mistyrose, color=red"
        else:
            attrs = f"label={label}"
        lines.append(f"  {node_id} [{attrs}];")
    for edge in graph["edges"]:
        source = json.dumps(edge["source"])
        target = json.dumps(edge["target"])
        attrs = f'label="{edge["order"]}"'
        if edge["status"] != "exact":
            attrs += ", color=red, style=dashed"
        lines.append(f"  {source} -> {target} [{attrs}];")
    lines.append("}")
    return "\n".join(lines) + "\n"


def render_outputs(source: Path) -> dict[str, str]:
    inventory, graph, csv_text, manifest = build_inventory(source)
    return {
        "repository-inventory.json": json.dumps(inventory, indent=2, sort_keys=True) + "\n",
        "repository-inventory.csv": csv_text,
        "include-graph.json": json.dumps(graph, indent=2, sort_keys=True) + "\n",
        "include-graph.dot": dot_graph(graph),
        "source-manifest.sha256": manifest,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    outputs = render_outputs(args.source.resolve())
    if args.check:
        stale = [
            name
            for name, expected in outputs.items()
            if not (args.output / name).is_file()
            or (args.output / name).read_text(encoding="utf-8") != expected
        ]
        if stale:
            raise SystemExit("stale or missing forensic artifacts: " + ", ".join(stale))
        return 0

    args.output.mkdir(parents=True, exist_ok=True)
    for name, content in outputs.items():
        (args.output / name).write_text(content, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
