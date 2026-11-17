"""Answers the GitHub API calls Runlet makes for the VM test's jobs.

Run 101 is a push to main (trusted); every other run is a pull request from
a fork (untrusted) whose only changed file is src/lib.rs.
The runners list holds the names in the runners file, one per line, each with
its line number as its ID.
"""

import http.server
import json
import sys
import urllib.parse

LOG = sys.argv[1]
RUNNERS = sys.argv[2]
REPOSITORY = {"id": 1, "owner": {"login": "org"}}
FORK = {"id": 2, "owner": {"login": "contributor"}}


def runners():
    try:
        with open(RUNNERS) as file:
            names = file.read().split()
    except FileNotFoundError:
        names = []
    return [{"id": index + 1, "name": name} for index, name in enumerate(names)]


def first_page(path):
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
    return query.get("page", ["1"]) == ["1"]


def response(method, path):
    if path == "/ping":
        return {"pong": True}
    if method == "POST" and path.startswith("/app/installations/"):
        return {"token": "fake-installation-token", "expires_at": "2099-01-01T00:00:00Z"}
    if method == "POST" and path.endswith("/actions/runners/registration-token"):
        return {"token": "fake-registration-token", "expires_at": "2099-01-01T00:00:00Z"}
    if path.startswith("/repos/org/project/actions/runs/"):
        run_id = int(path.rsplit("/", 1)[1])
        push = run_id == 101
        return {
            "id": run_id,
            "event": "push" if push else "pull_request",
            "head_branch": "main" if push else "feature",
            "head_sha": "fixture-commit",
            "repository": REPOSITORY,
            "head_repository": REPOSITORY if push else FORK,
        }
    if path.startswith("/repos/org/project/pulls?"):
        return [{"number": 9, "head": {"sha": "fixture-commit", "repo": {"id": 2}}}]
    if path.startswith("/repos/org/project/pulls/9/files?"):
        return [{"filename": "src/lib.rs"}] if first_page(path) else []
    if path.startswith("/repos/org/project/actions/runners?"):
        listed = runners() if first_page(path) else []
        return {"total_count": len(listed), "runners": listed}
    if method == "DELETE" and path.startswith("/repos/org/project/actions/runners/"):
        return {}
    return None


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def respond(self):
        with open(LOG, "a") as log:
            log.write(f"{self.command} {self.path}\n")
        data = response(self.command, self.path)
        body = json.dumps(data if data is not None else {"message": "Not Found"}).encode()
        self.send_response(200 if data is not None else 404)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = respond
    do_POST = respond
    do_DELETE = respond


http.server.ThreadingHTTPServer(("0.0.0.0", 9999), Handler).serve_forever()
