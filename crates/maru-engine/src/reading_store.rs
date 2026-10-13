use crate::engine::{EngineError, Result};
use maru_core::Rect;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadingArea {
    pub id: i64,
    pub exe: String,
    pub game: String,
    pub name: String,
    /// Fractions of the client area, never absolute desktop coordinates.
    pub region: Rect,
    pub reference_width: f32,
    pub reference_height: f32,
    pub selected: bool,
}

impl ReadingArea {
    pub fn rectangle(&self, window: Rect) -> Rect {
        let x = (window.x + self.region.x * window.w).round();
        let y = (window.y + self.region.y * window.h).round();
        let right = (window.x + (self.region.x + self.region.w) * window.w).round();
        let bottom = (window.y + (self.region.y + self.region.h) * window.h).round();
        Rect::new(x, y, right - x, bottom - y)
    }
    pub fn needs_check(&self, window: Rect) -> bool {
        (window.w - self.reference_width).abs() > 1.0
            || (window.h - self.reference_height).abs() > 1.0
    }
}

pub(crate) struct Store {
    conn: Connection,
}
fn err(e: impl std::fmt::Display) -> EngineError {
    let message = e.to_string();
    if message.contains("UNIQUE constraint failed") {
        return EngineError::Other("Saved reading areas: an area with that name already exists for this game; choose another name".into());
    }
    EngineError::Other(format!("Saved reading areas: {message}"))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS reading_areas (
            id INTEGER PRIMARY KEY AUTOINCREMENT, exe TEXT NOT NULL, game TEXT NOT NULL,
            name TEXT NOT NULL COLLATE NOCASE, x REAL NOT NULL, y REAL NOT NULL,
            w REAL NOT NULL, h REAL NOT NULL, reference_width REAL NOT NULL,
            reference_height REAL NOT NULL, selected INTEGER NOT NULL DEFAULT 0,
            UNIQUE(exe,name));",
        )
        .map_err(err)?;
        Ok(Self { conn })
    }
    pub fn list(&self) -> Result<Vec<ReadingArea>> {
        let mut stmt=self.conn.prepare("SELECT id,exe,game,name,x,y,w,h,reference_width,reference_height,selected FROM reading_areas ORDER BY game,name,id").map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ReadingArea {
                    id: r.get(0)?,
                    exe: r.get(1)?,
                    game: r.get(2)?,
                    name: r.get(3)?,
                    region: Rect::new(r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?),
                    reference_width: r.get(8)?,
                    reference_height: r.get(9)?,
                    selected: r.get(10)?,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }
    pub fn save(&mut self, mut area: ReadingArea) -> Result<ReadingArea> {
        let tx = self.conn.transaction().map_err(err)?;
        tx.execute(
            "UPDATE reading_areas SET selected=0 WHERE exe=?1",
            [&area.exe],
        )
        .map_err(err)?;
        if area.id == 0 {
            tx.execute("INSERT INTO reading_areas(exe,game,name,x,y,w,h,reference_width,reference_height,selected) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1)",params![area.exe,area.game,area.name,area.region.x,area.region.y,area.region.w,area.region.h,area.reference_width,area.reference_height]).map_err(err)?;
            area.id = tx.last_insert_rowid();
        } else {
            let n=tx.execute("UPDATE reading_areas SET game=?2,name=?3,x=?4,y=?5,w=?6,h=?7,reference_width=?8,reference_height=?9,selected=1 WHERE id=?1 AND exe=?10",params![area.id,area.game,area.name,area.region.x,area.region.y,area.region.w,area.region.h,area.reference_width,area.reference_height,area.exe]).map_err(err)?;
            if n != 1 {
                return Err(err("this area no longer exists"));
            }
        }
        tx.commit().map_err(err)?;
        area.selected = true;
        Ok(area)
    }
    pub fn select(&mut self, area: &ReadingArea) -> Result<()> {
        self.conn
            .execute(
                "UPDATE reading_areas SET selected=(id=?1) WHERE exe=?2",
                params![area.id, area.exe],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn remove(&mut self, id: i64) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM reading_areas WHERE id=?1", [id])
            .map_err(err)?
            != 1
        {
            return Err(err("this area no longer exists"));
        }
        Ok(())
    }
    pub fn rename(&mut self, id: i64, name: &str) -> Result<()> {
        if self
            .conn
            .execute(
                "UPDATE reading_areas SET name=?2 WHERE id=?1",
                params![id, name],
            )
            .map_err(err)?
            != 1
        {
            return Err(err("this area no longer exists"));
        }
        Ok(())
    }
}
