#!/usr/bin/env python3
"""Projects ボード（https://github.com/users/masa-iwm/projects/2）の Status を Issue・PR の状態に合わせる。

規則（Status の選択肢は Todo / In Progress / Done）:
  - 閉じた Issue、閉じた・マージした PR → Done。Issue に残った in-progress ラベルは外す
  - 開いている Issue → in-progress ラベルがあるか、開いている PR から参照されていれば In Progress、
    どちらも無ければ Todo（イベント時は PR 側のイベントで In Progress にする。毎日の sync が両方を見て揃える）
  - 開いている PR → In Progress。本文の Closes/Fixes/Resolves/Refs #N（別リポジトリは owner/repo#N）の
    開いている Issue も In Progress。Closes/Fixes/Resolves の Issue には in-progress ラベルも付ける
    （Refs はまとめの Issue を指すことが多く、PR が閉じた後も残るラベルは付けない）
  - 「Closes #1, #2」「Closes #1、#2」のように 1 つのキーワードに続く複数の番号も読む

使い方（環境変数 GH_TOKEN=classic PAT（project, repo）、REPO=owner/name）:
  project_status.py event   GitHub Actions のイベント（GITHUB_EVENT_NAME / GITHUB_EVENT_PATH）で 1 件を更新
  project_status.py sync    ボード上のこのリポジトリの全項目と、閉じた Issue の in-progress ラベルを一括で揃える
  project_status.py sync --dry-run   変更せずに表示だけ
"""
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

OWNER = os.environ.get("OWNER", "masa-iwm")
PROJECT = int(os.environ.get("PROJECT", "2"))
LABEL = "in-progress"
TODO, IN_PROGRESS, DONE = "Todo", "In Progress", "Done"

TOKEN = os.environ.get("GH_TOKEN", "")
REPO = os.environ.get("REPO") or os.environ.get("GITHUB_REPOSITORY", "")
DRY = "--dry-run" in sys.argv


def api(method, path, body=None):
    req = urllib.request.Request(
        "https://api.github.com" + path,
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(req) as r:
        data = r.read()
    return json.loads(data) if data else None


def graphql(query, **variables):
    res = api("POST", "/graphql", {"query": query, "variables": variables})
    if res.get("errors"):
        raise RuntimeError(json.dumps(res["errors"], ensure_ascii=False))
    return res["data"]


class Board:
    def __init__(self):
        p = graphql(
            """query($login: String!, $num: Int!) {
              user(login: $login) { projectV2(number: $num) {
                id
                field(name: "Status") { ... on ProjectV2SingleSelectField { id options { id name } } }
              } } }""",
            login=OWNER,
            num=PROJECT,
        )["user"]["projectV2"]
        self.id = p["id"]
        self.field = p["field"]["id"]
        self.options = {o["name"]: o["id"] for o in p["field"]["options"]}
        for s in (TODO, IN_PROGRESS, DONE):
            if s not in self.options:
                raise SystemExit(f"::error::Status に {s} が無い（{list(self.options)}）")

    def add(self, content_id):
        """ボードに追加して項目 id を返す（既にあれば既存の id）。"""
        return graphql(
            """mutation($pid: ID!, $cid: ID!) {
              addProjectV2ItemById(input: { projectId: $pid, contentId: $cid }) { item { id } } }""",
            pid=self.id,
            cid=content_id,
        )["addProjectV2ItemById"]["item"]["id"]

    def set(self, item_id, status):
        graphql(
            """mutation($pid: ID!, $iid: ID!, $fid: ID!, $oid: String!) {
              updateProjectV2ItemFieldValue(input: { projectId: $pid, itemId: $iid, fieldId: $fid,
                value: { singleSelectOptionId: $oid } }) { projectV2Item { id } } }""",
            pid=self.id,
            iid=item_id,
            fid=self.field,
            oid=self.options[status],
        )

    def set_content(self, content_id, status, label=""):
        if DRY:
            print(f"(dry-run) {label or content_id} → {status}")
            return
        try:
            iid = self.add(content_id)
        except RuntimeError as e:
            print(f"::warning::{label or content_id} をボードに追加できない: {e}")
            return
        self.set(iid, status)
        print(f"{label or content_id} → {status}")

    def items(self):
        """ボードの全項目（Issue・PR のみ）。"""
        cursor = None
        while True:
            d = graphql(
                """query($pid: ID!, $after: String) { node(id: $pid) { ... on ProjectV2 {
                  items(first: 100, after: $after) {
                    pageInfo { hasNextPage endCursor }
                    nodes {
                      id
                      fieldValueByName(name: "Status") { ... on ProjectV2ItemFieldSingleSelectValue { name } }
                      content {
                        __typename
                        ... on Issue { number state repository { nameWithOwner }
                                       labels(first: 50) { nodes { name } } }
                        ... on PullRequest { number state repository { nameWithOwner } }
                      }
                    } } } } }""",
                pid=self.id,
                after=cursor,
            )["node"]["items"]
            yield from d["nodes"]
            if not d["pageInfo"]["hasNextPage"]:
                return
            cursor = d["pageInfo"]["endCursor"]


def remove_label(repo, number):
    if DRY:
        print(f"(dry-run) {repo}#{number} から {LABEL} を外す")
        return
    try:
        api("DELETE", f"/repos/{repo}/issues/{number}/labels/{urllib.parse.quote(LABEL)}")
        print(f"{repo}#{number} から {LABEL} を外した")
    except urllib.error.HTTPError as e:
        if e.code != 404:  # 付いていなければ 404
            raise


def add_label(repo, number):
    if DRY:
        print(f"(dry-run) {repo}#{number} に {LABEL} を付ける")
        return
    api("POST", f"/repos/{repo}/issues/{number}/labels", {"labels": [LABEL]})
    print(f"{repo}#{number} に {LABEL} を付けた")


def issue_status(state, labels):
    if state.lower() != "open":
        return DONE
    return IN_PROGRESS if LABEL in labels else TODO


_REF = r"(?:[\w.-]+/[\w.-]+)?#\d+"
REF_RE = re.compile(
    r"\b(close[sd]?|fix(?:e[sd])?|resolve[sd]?|refs?)[: ]+(" + _REF + r"(?:\s*(?:[,、・]|and)\s*" + _REF + r")*)",
    re.I,
)


def parse_refs(body, repo):
    """PR 本文の参照を {(repo, 番号): 閉じる参照か} で返す（同じ Issue は閉じる参照を優先）。"""
    refs = {}
    for kw, group in REF_RE.findall(body or ""):
        closing = not kw.lower().startswith("ref")
        for ref in re.findall(_REF, group):
            r, num = ref.split("#")
            key = (r or repo, int(num))
            refs[key] = refs.get(key, False) or closing
    return refs


def on_event(board):
    name = os.environ["GITHUB_EVENT_NAME"]
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8") as f:
        ev = json.load(f)
    action = ev.get("action")

    if name == "issues":
        issue = ev["issue"]
        labels = [l["name"] for l in issue.get("labels", [])]
        if action in ("labeled", "unlabeled") and ev.get("label", {}).get("name") != LABEL:
            return
        if issue["state"] != "open" and LABEL in labels:
            remove_label(REPO, issue["number"])
        board.set_content(issue["node_id"], issue_status(issue["state"], labels), f"{REPO}#{issue['number']}")
        return

    if name == "pull_request":
        pr = ev["pull_request"]
        if pr["state"] != "open":
            board.set_content(pr["node_id"], DONE, f"{REPO}#{pr['number']} (PR)")
            if not pr.get("merged"):
                drop_auto_labels(board, pr)
            return
        board.set_content(pr["node_id"], IN_PROGRESS, f"{REPO}#{pr['number']} (PR)")
        for (repo, num), closing in parse_refs(pr.get("body"), REPO).items():
            try:
                it = api("GET", f"/repos/{repo}/issues/{num}")
            except urllib.error.HTTPError:
                print(f"::warning::{repo}#{num} が見つからない")
                continue
            if it["state"] != "open" or "pull_request" in it:  # 閉じた Issue は Done のまま
                continue
            if closing and LABEL not in [l["name"] for l in it.get("labels", [])]:
                add_label(repo, num)
            board.set_content(it["node_id"], IN_PROGRESS, f"{repo}#{num}")
        return

    print(f"対象外のイベント: {name}")


def last_labeled_at(repo, number):
    """Issue に in-progress ラベルが最後に付いた時刻（ISO 8601 の文字列、無ければ None）。"""
    last, page = None, 1
    while True:
        q = urllib.parse.urlencode({"per_page": 100, "page": page})
        rows = api("GET", f"/repos/{repo}/issues/{number}/events?{q}")
        for e in rows:
            if e.get("event") == "labeled" and (e.get("label") or {}).get("name") == LABEL:
                last = max(last or "", e["created_at"])
        if len(rows) < 100:
            return last
        page += 1


def drop_auto_labels(board, pr):
    """マージせずに閉じた PR が閉じる参照をしていた Issue から、その PR が付けたとみられる in-progress を外す。

    スレッドは着手した時（PR より前）に手でラベルを付けるので、最後に付いたのが PR を開いた時より後なら
    PR の自動付与とみなして外す。PR より前に付いていたもの（手で付けた着手中）と、ほかに開いている PR が
    閉じる参照をしている Issue はそのまま。
    """
    refs = {k: v for k, v in parse_refs(pr.get("body"), REPO).items() if v}
    if not refs:
        return
    others = open_pr_refs(sorted({r for r, _ in refs} | {REPO}))
    for (repo, num) in sorted(refs):
        if others.get((repo, num)):
            print(f"{repo}#{num}: ほかに開いている PR が閉じる参照をしているので {LABEL} を残す")
            continue
        try:
            it = api("GET", f"/repos/{repo}/issues/{num}")
        except urllib.error.HTTPError:
            continue
        if it["state"] != "open" or "pull_request" in it:
            continue
        if LABEL not in [l["name"] for l in it.get("labels", [])]:
            continue
        at = last_labeled_at(repo, num)
        if at is None or at < pr["created_at"]:
            print(f"{repo}#{num}: {LABEL} は PR より前に付いていたので残す")
            continue
        remove_label(repo, num)
        board.set_content(it["node_id"], TODO, f"{repo}#{num}")


def open_pr_refs(repos):
    """開いている PR の本文が参照する Issue の {(repo, 番号): 閉じる参照か}。"""
    refs = {}
    for repo in repos:
        page = 1
        while True:
            q = urllib.parse.urlencode({"state": "open", "per_page": 100, "page": page})
            rows = api("GET", f"/repos/{repo}/pulls?{q}")
            for pr in rows:
                for key, closing in parse_refs(pr.get("body"), repo).items():
                    refs[key] = refs.get(key, False) or closing
            if len(rows) < 100:
                break
            page += 1
    return refs


def sync(board):
    changed = 0
    items = list(board.items())
    # 開いている PR から参照された Issue は In Progress（イベント時と同じ）。
    # private の PR が公開側の Issue を参照するので、ボード上の全リポジトリの PR を見る
    repos = sorted({(i.get("content") or {}).get("repository", {}).get("nameWithOwner")
                    for i in items} - {None} | {REPO})
    referenced = open_pr_refs(repos)
    seen = set()
    for item in items:
        c = item.get("content") or {}
        kind = c.get("__typename")
        if kind not in ("Issue", "PullRequest") or c["repository"]["nameWithOwner"] != REPO:
            continue
        cur = (item.get("fieldValueByName") or {}).get("name")
        tag = f"{REPO}#{c['number']}" + (" (PR)" if kind == "PullRequest" else "")
        if kind == "Issue":
            labels = [l["name"] for l in c["labels"]["nodes"]]
            seen.add(c["number"])
            if c["state"] != "OPEN" and LABEL in labels:
                remove_label(REPO, c["number"])
                changed += 1
            if c["state"] == "OPEN" and LABEL not in labels and referenced.get((REPO, c["number"])):
                add_label(REPO, c["number"])
                labels.append(LABEL)
                changed += 1
            want = issue_status(c["state"], labels)
            if want == TODO and (REPO, c["number"]) in referenced:
                want = IN_PROGRESS
        else:
            want = IN_PROGRESS if c["state"] == "OPEN" else DONE
        if cur != want:
            print(f"{tag}: {cur} → {want}")
            if not DRY:
                board.set(item["id"], want)
            changed += 1

    # ボードに無い Issue で、開いている PR が閉じる参照をしているものはボードに足してラベルを付ける
    for (repo, num), closing in sorted(referenced.items()):
        if repo != REPO or not closing or num in seen:
            continue
        try:
            it = api("GET", f"/repos/{REPO}/issues/{num}")
        except urllib.error.HTTPError:
            continue
        if it["state"] != "open" or "pull_request" in it:
            continue
        if LABEL not in [l["name"] for l in it.get("labels", [])]:
            add_label(REPO, num)
        board.set_content(it["node_id"], IN_PROGRESS, f"{REPO}#{num}")
        changed += 1

    # ボードに無い閉じた Issue のラベルも外す（外すと一覧がずれるので、先に全部集める）
    stale, page = [], 1
    while True:
        q = urllib.parse.urlencode({"state": "closed", "labels": LABEL, "per_page": 100, "page": page})
        rows = api("GET", f"/repos/{REPO}/issues?{q}")
        stale += [it["number"] for it in rows if "pull_request" not in it]
        if len(rows) < 100:
            break
        page += 1
    for num in stale:
        remove_label(REPO, num)
        changed += 1
    print(f"変更 {changed} 件" + ("（dry-run）" if DRY else ""))


def main():
    if not TOKEN:
        print("::warning::ADD_TO_PROJECT_PAT が無いので更新しない")
        return
    board = Board()
    if len(sys.argv) > 1 and sys.argv[1] == "sync":
        sync(board)
    else:
        on_event(board)


if __name__ == "__main__":
    main()
