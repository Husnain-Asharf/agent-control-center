use acc_protocol::{Approval, ApprovalState, EventRecord, Project, Session, SessionStatus, StatusChange};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct NewEvent {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: String,
    pub actor: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AllowRule {
    pub tool: String,
    pub prefix: Option<String>,
}

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    session_id TEXT,
    project_id TEXT,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_session ON events(session_id, seq);
CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS approvals (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, state TEXT NOT NULL, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS rules (
    project_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    prefix TEXT NOT NULL DEFAULT '',
    UNIQUE(project_id, tool, prefix)
);
";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn state_str(s: &ApprovalState) -> Result<String> {
    Ok(serde_json::to_value(s)?.as_str().unwrap_or_default().to_string())
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", 1)?;
        Ok(Store { conn })
    }

    /// `ts` is milliseconds since the unix epoch.
    pub fn append_event(&self, e: NewEvent) -> Result<EventRecord> {
        let ts = now_ms();
        let payload = serde_json::to_string(&e.payload)?;
        self.conn.execute(
            "INSERT INTO events (ts, session_id, project_id, kind, actor, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![ts, e.session_id, e.project_id, e.kind, e.actor, payload],
        )?;
        Ok(EventRecord {
            seq: self.conn.last_insert_rowid(),
            ts,
            session_id: e.session_id,
            project_id: e.project_id,
            kind: e.kind,
            actor: e.actor,
            payload: e.payload,
        })
    }

    /// Newest first.
    pub fn list_events(&self, session_id: Option<&str>, limit: u32) -> Result<Vec<EventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, session_id, project_id, kind, actor, payload FROM events
             WHERE (?1 IS NULL OR session_id = ?1) ORDER BY seq DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, String>(6)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, ts, session_id, project_id, kind, actor, payload) = row?;
            out.push(EventRecord { seq, ts, session_id, project_id, kind, actor, payload: serde_json::from_str(&payload)? });
        }
        Ok(out)
    }

    pub fn upsert_project(&self, p: &Project) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects (id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            params![p.id, serde_json::to_string(p)?],
        )?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        self.load_all("SELECT data FROM projects ORDER BY rowid")
    }

    pub fn upsert_session(&self, s: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, project_id, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET project_id = excluded.project_id, data = excluded.data",
            params![s.id, s.project_id, serde_json::to_string(s)?],
        )?;
        Ok(())
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        self.load_all("SELECT data FROM sessions ORDER BY rowid")
    }

    pub fn upsert_approval(&self, a: &Approval) -> Result<()> {
        self.conn.execute(
            "INSERT INTO approvals (id, session_id, state, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET session_id = excluded.session_id, state = excluded.state, data = excluded.data",
            params![a.id, a.session_id, state_str(&a.state)?, serde_json::to_string(a)?],
        )?;
        Ok(())
    }

    pub fn pending_approvals(&self) -> Result<Vec<Approval>> {
        self.load_all("SELECT data FROM approvals WHERE state = 'pending' ORDER BY rowid")
    }

    pub fn add_rule(&self, project_id: &str, r: &AllowRule) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO rules (project_id, tool, prefix) VALUES (?1, ?2, ?3)",
            params![project_id, r.tool, r.prefix.as_deref().unwrap_or("")],
        )?;
        Ok(())
    }

    pub fn rules(&self, project_id: &str) -> Result<Vec<AllowRule>> {
        let mut stmt = self.conn.prepare("SELECT tool, prefix FROM rules WHERE project_id = ?1 ORDER BY rowid")?;
        let rows = stmt.query_map(params![project_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (tool, prefix) = row?;
            out.push(AllowRule { tool, prefix: if prefix.is_empty() { None } else { Some(prefix) } });
        }
        Ok(out)
    }

    /// Run at daemon start: sessions that were live when the daemon died become
    /// Stopped and pending approvals become Cancelled. Returns what changed.
    pub fn mark_orphans_stopped(&self, now: i64) -> Result<(Vec<Session>, Vec<Approval>)> {
        let mut changed_sessions = Vec::new();
        for mut s in self.sessions()? {
            if matches!(s.status, SessionStatus::Stopped | SessionStatus::Done | SessionStatus::Failed) {
                continue;
            }
            s.status = SessionStatus::Stopped;
            s.status_since = now;
            s.history.push(StatusChange { status: SessionStatus::Stopped, at: now });
            if s.ended_at.is_none() {
                s.ended_at = Some(now);
            }
            self.upsert_session(&s)?;
            changed_sessions.push(s);
        }
        let mut changed_approvals = Vec::new();
        for mut a in self.pending_approvals()? {
            a.state = ApprovalState::Cancelled;
            self.upsert_approval(&a)?;
            changed_approvals.push(a);
        }
        Ok((changed_sessions, changed_approvals))
    }

    fn load_all<T: serde::de::DeserializeOwned>(&self, sql: &str) -> Result<Vec<T>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acc_protocol::{ApprovalKind, SessionMode};
    use serde_json::json;

    fn ev(session: Option<&str>, kind: &str) -> NewEvent {
        NewEvent { session_id: session.map(String::from), project_id: Some("p1".into()), kind: kind.into(), actor: "user".into(), payload: json!({"k": kind}) }
    }
    fn session(id: &str, status: SessionStatus) -> Session {
        Session {
            id: id.into(), project_id: "p1".into(), agent: "claude".into(), title: "t".into(), branch: "agent/x".into(),
            worktree: "/w".into(), mode: SessionMode::Worktree, stay_on_base: false, usage: None, status, status_since: 10, started_at: 5,
            ended_at: None, exit_code: None,
            history: vec![StatusChange { status: SessionStatus::Starting, at: 5 }],
        }
    }
    fn approval(id: &str, state: ApprovalState) -> Approval {
        Approval {
            id: id.into(), session_id: "s1".into(), tool: "Bash".into(), summary: "ls".into(), input: json!({"command": "ls"}),
            created_at: 1, state, decided_by: None, protected: false, rule_preview: Some("`ls` commands".into()),
            kind: ApprovalKind::Permission, branch: None,
        }
    }

    #[test]
    fn events_are_listed_newest_first_with_limit() {
        let s = Store::open_in_memory().unwrap();
        for k in ["a", "b", "c"] { s.append_event(ev(None, k)).unwrap(); }
        let got = s.list_events(None, 2).unwrap();
        assert_eq!(got.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3, 2]);
        assert_eq!(got[0].kind, "c");
        assert_eq!(got[0].payload, json!({"k": "c"}));
    }

    #[test]
    fn event_ts_is_milliseconds() {
        let s = Store::open_in_memory().unwrap();
        let e = s.append_event(ev(None, "a")).unwrap();
        assert!(e.ts > 1_000_000_000_000, "ts {} is not on the ms scale", e.ts);
        assert_eq!(s.list_events(None, 1).unwrap()[0].ts, e.ts);
    }

    #[test]
    fn events_filter_by_session() {
        let s = Store::open_in_memory().unwrap();
        s.append_event(ev(Some("s1"), "a")).unwrap();
        s.append_event(ev(Some("s2"), "b")).unwrap();
        s.append_event(ev(Some("s1"), "c")).unwrap();
        let got = s.list_events(Some("s1"), 10).unwrap();
        assert_eq!(got.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["c", "a"]);
    }

    #[test]
    fn upsert_session_replaces() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_session(&session("s1", SessionStatus::Working)).unwrap();
        s.upsert_session(&session("s1", SessionStatus::Done)).unwrap();
        let all = s.sessions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, SessionStatus::Done);
    }

    #[test]
    fn pending_approvals_excludes_resolved() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_approval(&approval("a1", ApprovalState::Pending)).unwrap();
        s.upsert_approval(&approval("a2", ApprovalState::Pending)).unwrap();
        s.upsert_approval(&approval("a2", ApprovalState::Allowed)).unwrap();
        s.upsert_approval(&approval("a3", ApprovalState::Denied)).unwrap();
        let ids: Vec<_> = s.pending_approvals().unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, vec!["a1"]);
    }

    #[test]
    fn rules_roundtrip_and_dedupe() {
        let s = Store::open_in_memory().unwrap();
        let r = AllowRule { tool: "Bash".into(), prefix: Some("git status".into()) };
        let bare = AllowRule { tool: "Read".into(), prefix: None };
        s.add_rule("p1", &r).unwrap();
        s.add_rule("p1", &r).unwrap();
        s.add_rule("p1", &bare).unwrap();
        s.add_rule("p1", &bare).unwrap();
        s.add_rule("p2", &r).unwrap();
        let got = s.rules("p1").unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&r) && got.contains(&bare));
        assert_eq!(s.rules("p2").unwrap().len(), 1);
    }

    #[test]
    fn store_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acc.db");
        {
            let s = Store::open(&path).unwrap();
            s.append_event(ev(Some("s1"), "a")).unwrap();
            s.upsert_session(&session("s1", SessionStatus::Idle)).unwrap();
            s.upsert_project(&Project { id: "p1".into(), name: "n".into(), path: "/p".into(), host: None, base_branch: "main".into(), added_at: 1 }).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.list_events(None, 10).unwrap().len(), 1);
        assert_eq!(s.sessions().unwrap()[0].status, SessionStatus::Idle);
        assert_eq!(s.projects().unwrap()[0].name, "n");
        assert_eq!(s.append_event(ev(None, "b")).unwrap().seq, 2);
    }

    #[test]
    fn mark_orphans_stopped_changes_live_sessions() {
        let s = Store::open_in_memory().unwrap();
        s.upsert_session(&session("w", SessionStatus::Working)).unwrap();
        s.upsert_session(&session("n", SessionStatus::NeedsYou)).unwrap();
        s.upsert_session(&session("d", SessionStatus::Done)).unwrap();
        s.upsert_approval(&approval("a1", ApprovalState::Pending)).unwrap();
        s.upsert_approval(&approval("a2", ApprovalState::Allowed)).unwrap();

        let (sessions, approvals) = s.mark_orphans_stopped(100).unwrap();
        let mut ids: Vec<_> = sessions.iter().map(|x| x.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["n", "w"]);
        for x in &sessions {
            assert_eq!(x.status, SessionStatus::Stopped);
            assert_eq!(x.status_since, 100);
            assert_eq!(x.ended_at, Some(100));
            assert_eq!(x.history.last().unwrap(), &StatusChange { status: SessionStatus::Stopped, at: 100 });
        }
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].id, "a1");
        assert_eq!(approvals[0].state, ApprovalState::Cancelled);

        let stored = s.sessions().unwrap();
        assert_eq!(stored.iter().find(|x| x.id == "d").unwrap().status, SessionStatus::Done);
        assert_eq!(stored.iter().find(|x| x.id == "w").unwrap().status, SessionStatus::Stopped);
        assert!(s.pending_approvals().unwrap().is_empty());
    }
}
