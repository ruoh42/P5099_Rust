use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use p5099_core::{
    club_protocol::{
        CLUB_CREW_PAGE_SIZE, CLUB_GRADE_MANAGER, CLUB_GRADE_MASTER, CLUB_GRADE_MEMBER,
        CLUB_LIST_PAGE_SIZE, CLUB_MAX_MEMBERS, CLUB_RESULT_OK, CLUB_RESULT_REFRESH,
        CREATE_CLUB_ALREADY_JOINED, CREATE_CLUB_FAILED, CREATE_CLUB_NAME_DUPLICATE,
        CREATE_CLUB_SUCCESS, ClubBankView, ClubInfoView, ClubMemberView, ClubSupportRecord,
        ClubWaitingCrewView, JOIN_CLUB_CREW_FULL, JOIN_CLUB_FAILED, JOIN_CLUB_JOINED,
        JOIN_CLUB_WAITING, KICK_RESULT_MASTER,
    },
    login::LegacyTime,
    nickname::canonical_nickname_key,
};
use serde::{Deserialize, Serialize};

pub(crate) const CLUB_DIRECTORY_FILE_NAME: &str = "Clubs.json";
const FIRST_CLUB_CODE: u32 = 10_001;
const MAX_CLUB_MEMBERS: usize = CLUB_MAX_MEMBERS as usize;
const MAX_WAITING_CREW: usize = 50;
pub(crate) const LEAVE_RESTRICTION_SECONDS: u64 = 24 * 60 * 60;
pub(crate) const INITIAL_CLUB_CAPITAL: u32 = 100;
const MAX_SUPPORT_HISTORY: usize = 20;

const fn initial_club_capital() -> u32 {
    INITIAL_CLUB_CAPITAL
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ClubBook {
    next_code: u32,
    clubs: Vec<StoredClub>,
    #[serde(default)]
    departures: Vec<StoredDeparture>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredDeparture {
    nickname: String,
    left_at: u64,
}

impl ClubBook {
    fn record_departure(&mut self, nickname: &str, now_unix: u64) {
        self.departures.retain(|departure| {
            !same_rider(&departure.nickname, nickname)
                && now_unix.saturating_sub(departure.left_at) < LEAVE_RESTRICTION_SECONDS
        });
        self.departures.push(StoredDeparture {
            nickname: nickname.to_owned(),
            left_at: now_unix,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredClub {
    code: u32,
    name: String,
    intro: String,
    mark_logo: u32,
    mark_line: u32,
    created_days: u16,
    created_quarter_seconds: u16,
    auto_join: bool,
    members: Vec<StoredMember>,
    #[serde(default)]
    waiting: Vec<StoredWaiting>,
    #[serde(default = "initial_club_capital")]
    capital: u32,
    #[serde(default)]
    supports: Vec<StoredSupport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredSupport {
    nickname: String,
    amount: u32,
    days: u16,
    quarter_seconds: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredMember {
    nickname: String,
    user_no: u32,
    rp: u32,
    grade: u16,
    joined_days: u16,
    joined_quarter_seconds: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredWaiting {
    nickname: String,
    user_no: u32,
    rp: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ClubActor<'a> {
    pub nickname: &'a str,
    pub user_no: u32,
    pub rp: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClubMembership {
    pub info: ClubInfoView,
    pub me: ClubMemberView,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinOutcome {
    pub club_code: u32,
    pub club_name: String,
    pub grade: u16,
    pub result: u32,
}

#[derive(Debug)]
pub(crate) struct ClubDirectory {
    path: Option<PathBuf>,
    book: Mutex<ClubBook>,
}

impl Default for ClubDirectory {
    fn default() -> Self {
        Self::in_memory()
    }
}

fn same_rider(left: &str, right: &str) -> bool {
    canonical_nickname_key(left) == canonical_nickname_key(right)
}

fn time(days: u16, quarter_seconds: u16) -> LegacyTime {
    LegacyTime {
        days_since_1900: days,
        quarter_seconds,
    }
}

impl StoredClub {
    fn master(&self) -> Option<&StoredMember> {
        self.members
            .iter()
            .find(|member| member.grade == CLUB_GRADE_MASTER)
    }

    fn info(&self) -> ClubInfoView {
        let master = self.master();
        ClubInfoView {
            club_code: self.code,
            name: self.name.clone(),
            created: time(self.created_days, self.created_quarter_seconds),
            master_user_no: master.map_or(0, |member| member.user_no),
            master_nickname: master.map_or_else(String::new, |member| member.nickname.clone()),
            intro: self.intro.clone(),
            mark_logo: self.mark_logo,
            mark_line: self.mark_line,
            member_count: u32::try_from(self.members.len()).unwrap_or(u32::MAX),
        }
    }

    fn member_view(&self, member: &StoredMember) -> ClubMemberView {
        ClubMemberView {
            club_code: self.code,
            user_no: member.user_no,
            nickname: member.nickname.clone(),
            rp: member.rp,
            grade: member.grade,
            joined: time(member.joined_days, member.joined_quarter_seconds),
        }
    }

    fn member_index(&self, nickname: &str) -> Option<usize> {
        self.members
            .iter()
            .position(|member| same_rider(&member.nickname, nickname))
    }

    fn bank_view(&self, nickname: &str) -> ClubBankView {
        let mut totals: Vec<(&str, u32)> = Vec::new();
        for support in &self.supports {
            match totals
                .iter_mut()
                .find(|(name, _)| same_rider(name, &support.nickname))
            {
                Some((_, total)) => *total = total.saturating_add(support.amount),
                None => totals.push((&support.nickname, support.amount)),
            }
        }
        let my_total = totals
            .iter()
            .find(|(name, _)| same_rider(name, nickname))
            .map_or(0, |(_, total)| *total);
        let top_supporter = totals
            .iter()
            .max_by_key(|(_, total)| *total)
            .map_or_else(String::new, |(name, _)| (*name).to_owned());
        let history = self
            .supports
            .iter()
            .rev()
            .take(MAX_SUPPORT_HISTORY)
            .map(|support| ClubSupportRecord {
                date: time(support.days, support.quarter_seconds),
                nickname: support.nickname.clone(),
            })
            .collect();
        ClubBankView {
            capital: self.capital,
            my_total,
            top_supporter,
            history,
        }
    }

    fn can_manage(&self, nickname: &str) -> bool {
        self.member_index(nickname)
            .is_some_and(|index| self.members[index].grade >= CLUB_GRADE_MANAGER)
    }
}

fn page<T: Clone>(items: &[T], page: u32, size: usize) -> Vec<T> {
    let start = usize::try_from(page)
        .unwrap_or(usize::MAX)
        .saturating_mul(size);
    items.iter().skip(start).take(size).cloned().collect()
}

fn matches_filter(club: &StoredClub, name_filter: &str, master_filter: &str) -> bool {
    let name_ok = name_filter.is_empty()
        || club
            .name
            .to_lowercase()
            .contains(&name_filter.to_lowercase());
    let master_ok = master_filter.is_empty()
        || club
            .master()
            .is_some_and(|master| same_rider(&master.nickname, master_filter));
    name_ok && master_ok
}

impl ClubDirectory {
    #[must_use]
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            book: Mutex::new(ClubBook {
                next_code: FIRST_CLUB_CODE,
                ..ClubBook::default()
            }),
        }
    }

    #[must_use]
    pub(crate) fn open(profile_root: &Path) -> Self {
        let path = profile_root.join(CLUB_DIRECTORY_FILE_NAME);
        let book = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<ClubBook>(&bytes) {
                Ok(mut book) => {
                    let highest = book.clubs.iter().map(|club| club.code).max().unwrap_or(0);
                    book.next_code = book.next_code.max(FIRST_CLUB_CODE).max(highest + 1);
                    book
                }
                Err(error) => {
                    let aside = path.with_extension("json.unreadable");
                    tracing::error!(path = %path.display(), %error, aside = %aside.display(), "club directory is unreadable; keeping it aside and starting empty");
                    let _ = fs::rename(&path, &aside);
                    ClubBook::default()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => ClubBook::default(),
            Err(error) => {
                tracing::error!(path = %path.display(), %error, "cannot read the club directory; starting empty");
                ClubBook::default()
            }
        };
        let book = ClubBook {
            next_code: book.next_code.max(FIRST_CLUB_CODE),
            ..book
        };
        Self {
            path: Some(path),
            book: Mutex::new(book),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ClubBook> {
        self.book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn persist(&self, book: &ClubBook) {
        let Some(path) = &self.path else {
            return;
        };
        let result = serde_json::to_vec_pretty(book)
            .map_err(io::Error::other)
            .and_then(|bytes| {
                let temporary = path.with_extension("json.tmp");
                fs::write(&temporary, bytes)?;
                fs::rename(&temporary, path)
            });
        if let Err(error) = result {
            tracing::error!(path = %path.display(), %error, "failed to save the club directory");
        }
    }

    fn club_of<'a>(book: &'a ClubBook, nickname: &str) -> Option<&'a StoredClub> {
        book.clubs
            .iter()
            .find(|club| club.member_index(nickname).is_some())
    }

    fn club_of_mut<'a>(book: &'a mut ClubBook, nickname: &str) -> Option<&'a mut StoredClub> {
        book.clubs
            .iter_mut()
            .find(|club| club.member_index(nickname).is_some())
    }

    pub(crate) fn membership(&self, actor: ClubActor<'_>) -> Option<ClubMembership> {
        let mut book = self.lock();
        let club = Self::club_of_mut(&mut book, actor.nickname)?;
        let index = club.member_index(actor.nickname)?;
        let changed = {
            let member = &mut club.members[index];
            let changed = member.user_no != actor.user_no || member.rp != actor.rp;
            member.user_no = actor.user_no;
            member.rp = actor.rp;
            changed
        };
        let membership = ClubMembership {
            info: club.info(),
            me: club.member_view(&club.members[index]),
        };
        if changed {
            let snapshot = book.clone();
            self.persist(&snapshot);
        }
        Some(membership)
    }

    pub(crate) fn club_info_of(&self, nickname: &str) -> Option<ClubInfoView> {
        Self::club_of(&self.lock(), nickname).map(StoredClub::info)
    }

    pub(crate) fn waiting_club_of(&self, nickname: &str) -> Option<(u32, String)> {
        self.lock()
            .clubs
            .iter()
            .find(|club| {
                club.waiting
                    .iter()
                    .any(|waiting| same_rider(&waiting.nickname, nickname))
            })
            .map(|club| (club.code, club.name.clone()))
    }

    pub(crate) fn bank(&self, nickname: &str) -> Option<ClubBankView> {
        Self::club_of(&self.lock(), nickname).map(|club| club.bank_view(nickname))
    }

    pub(crate) fn support(
        &self,
        nickname: &str,
        amount: u32,
        now: LegacyTime,
    ) -> Option<ClubBankView> {
        self.mutate(|book| match Self::club_of_mut(book, nickname) {
            Some(club) => {
                let member = club.member_index(nickname).map_or_else(
                    || nickname.to_owned(),
                    |index| club.members[index].nickname.clone(),
                );
                club.capital = club.capital.saturating_add(amount);
                club.supports.push(StoredSupport {
                    nickname: member,
                    amount,
                    days: now.days_since_1900,
                    quarter_seconds: now.quarter_seconds,
                });
                (Some(club.bank_view(nickname)), true)
            }
            None => (None, false),
        })
    }

    pub(crate) fn leave_restricted(&self, nickname: &str, now_unix: u64) -> bool {
        self.lock().departures.iter().any(|departure| {
            same_rider(&departure.nickname, nickname)
                && now_unix.saturating_sub(departure.left_at) < LEAVE_RESTRICTION_SECONDS
        })
    }

    pub(crate) fn club_info(&self, club_code: u32) -> Option<ClubInfoView> {
        self.lock()
            .clubs
            .iter()
            .find(|club| club.code == club_code)
            .map(StoredClub::info)
    }

    pub(crate) fn auto_join(&self, club_code: u32) -> Option<bool> {
        self.lock()
            .clubs
            .iter()
            .find(|club| club.code == club_code)
            .map(|club| club.auto_join)
    }

    pub(crate) fn search(
        &self,
        name_filter: &str,
        master_filter: &str,
        page_index: u32,
    ) -> Vec<ClubInfoView> {
        let book = self.lock();
        let matching = book
            .clubs
            .iter()
            .filter(|club| matches_filter(club, name_filter, master_filter))
            .map(StoredClub::info)
            .collect::<Vec<_>>();
        page(&matching, page_index, CLUB_LIST_PAGE_SIZE)
    }

    pub(crate) fn count(&self, name_filter: &str, master_filter: &str) -> u32 {
        let book = self.lock();
        let count = book
            .clubs
            .iter()
            .filter(|club| matches_filter(club, name_filter, master_filter))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    pub(crate) fn waiting_count(&self, club_code: u32) -> (u32, u32) {
        let book = self.lock();
        let current = book
            .clubs
            .iter()
            .find(|club| club.code == club_code)
            .map_or(0, |club| club.waiting.len());
        (
            u32::try_from(current).unwrap_or(u32::MAX),
            u32::try_from(MAX_WAITING_CREW).unwrap_or(u32::MAX),
        )
    }

    pub(crate) fn crew_page(&self, club_code: u32, page_index: u32) -> (Vec<ClubMemberView>, u32) {
        let book = self.lock();
        let Some(club) = book.clubs.iter().find(|club| club.code == club_code) else {
            return (Vec::new(), 0);
        };
        let mut members = club.members.clone();
        members.sort_by_key(|member| std::cmp::Reverse(member.grade));
        let rows = page(&members, page_index, CLUB_CREW_PAGE_SIZE)
            .iter()
            .map(|member| club.member_view(member))
            .collect();
        (rows, u32::try_from(members.len()).unwrap_or(u32::MAX))
    }

    pub(crate) fn waiting_page(
        &self,
        club_code: u32,
        page_index: u32,
    ) -> (Vec<ClubWaitingCrewView>, u32) {
        let book = self.lock();
        let Some(club) = book.clubs.iter().find(|club| club.code == club_code) else {
            return (Vec::new(), 0);
        };
        let rows = page(&club.waiting, page_index, CLUB_CREW_PAGE_SIZE)
            .into_iter()
            .map(|waiting| ClubWaitingCrewView {
                user_no: waiting.user_no,
                nickname: waiting.nickname,
                rp: waiting.rp,
            })
            .collect();
        (rows, u32::try_from(club.waiting.len()).unwrap_or(u32::MAX))
    }

    pub(crate) fn create(
        &self,
        actor: ClubActor<'_>,
        name: &str,
        intro: &str,
        mark_logo: u32,
        mark_line: u32,
        now: LegacyTime,
    ) -> (u32, u32) {
        let name = name.trim();
        if name.is_empty() {
            return (0, CREATE_CLUB_FAILED);
        }
        let mut book = self.lock();
        if Self::club_of(&book, actor.nickname).is_some() {
            return (0, CREATE_CLUB_ALREADY_JOINED);
        }
        if book
            .clubs
            .iter()
            .any(|club| club.name.to_lowercase() == name.to_lowercase())
        {
            return (0, CREATE_CLUB_NAME_DUPLICATE);
        }
        let code = book.next_code.max(FIRST_CLUB_CODE);
        book.next_code = code + 1;
        for club in &mut book.clubs {
            club.waiting
                .retain(|waiting| !same_rider(&waiting.nickname, actor.nickname));
        }
        book.clubs.push(StoredClub {
            code,
            name: name.to_owned(),
            intro: intro.to_owned(),
            mark_logo,
            mark_line,
            created_days: now.days_since_1900,
            created_quarter_seconds: now.quarter_seconds,
            auto_join: true,
            capital: INITIAL_CLUB_CAPITAL,
            supports: Vec::new(),
            members: vec![StoredMember {
                nickname: actor.nickname.to_owned(),
                user_no: actor.user_no,
                rp: actor.rp,
                grade: CLUB_GRADE_MASTER,
                joined_days: now.days_since_1900,
                joined_quarter_seconds: now.quarter_seconds,
            }],
            waiting: Vec::new(),
        });
        let snapshot = book.clone();
        drop(book);
        self.persist(&snapshot);
        (code, CREATE_CLUB_SUCCESS)
    }

    pub(crate) fn join(
        &self,
        actor: ClubActor<'_>,
        club_code: u32,
        now: LegacyTime,
    ) -> JoinOutcome {
        let mut book = self.lock();
        let failed = |result| JoinOutcome {
            club_code,
            club_name: String::new(),
            grade: 0,
            result,
        };
        if Self::club_of(&book, actor.nickname).is_some() {
            return failed(JOIN_CLUB_FAILED);
        }
        for club in &mut book.clubs {
            club.waiting
                .retain(|waiting| !same_rider(&waiting.nickname, actor.nickname));
        }
        let Some(club) = book.clubs.iter_mut().find(|club| club.code == club_code) else {
            return failed(JOIN_CLUB_FAILED);
        };
        let outcome = if club.members.len() >= MAX_CLUB_MEMBERS {
            JoinOutcome {
                club_code,
                club_name: club.name.clone(),
                grade: 0,
                result: JOIN_CLUB_CREW_FULL,
            }
        } else if club.auto_join {
            club.members.push(StoredMember {
                nickname: actor.nickname.to_owned(),
                user_no: actor.user_no,
                rp: actor.rp,
                grade: CLUB_GRADE_MEMBER,
                joined_days: now.days_since_1900,
                joined_quarter_seconds: now.quarter_seconds,
            });
            JoinOutcome {
                club_code,
                club_name: club.name.clone(),
                grade: CLUB_GRADE_MEMBER,
                result: JOIN_CLUB_JOINED,
            }
        } else {
            club.waiting.push(StoredWaiting {
                nickname: actor.nickname.to_owned(),
                user_no: actor.user_no,
                rp: actor.rp,
            });
            JoinOutcome {
                club_code,
                club_name: club.name.clone(),
                grade: 0,
                result: JOIN_CLUB_WAITING,
            }
        };
        let snapshot = book.clone();
        drop(book);
        self.persist(&snapshot);
        outcome
    }

    pub(crate) fn cancel_waiting(&self, nickname: &str) {
        let mut book = self.lock();
        let before = book.clone();
        for club in &mut book.clubs {
            club.waiting
                .retain(|waiting| !same_rider(&waiting.nickname, nickname));
        }
        if *book != before {
            let snapshot = book.clone();
            drop(book);
            self.persist(&snapshot);
        }
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut ClubBook) -> (T, bool)) -> T {
        let mut book = self.lock();
        let (value, changed) = change(&mut book);
        if changed {
            let snapshot = book.clone();
            drop(book);
            self.persist(&snapshot);
        }
        value
    }

    pub(crate) fn leave(&self, nickname: &str, now_unix: u64) -> u32 {
        self.mutate(|book| {
            let Some(position) = book
                .clubs
                .iter()
                .position(|club| club.member_index(nickname).is_some())
            else {
                return (CLUB_RESULT_REFRESH, false);
            };
            let club = &mut book.clubs[position];
            let index = club.member_index(nickname).expect("member was just found");
            let leaving = club.members.remove(index);
            if club.members.is_empty() {
                book.clubs.remove(position);
            } else if leaving.grade == CLUB_GRADE_MASTER {
                let successor = club
                    .members
                    .iter_mut()
                    .max_by_key(|member| member.grade)
                    .expect("the club still has members");
                successor.grade = CLUB_GRADE_MASTER;
            }
            book.record_departure(nickname, now_unix);
            (CLUB_RESULT_OK, true)
        })
    }

    pub(crate) fn break_up(&self, nickname: &str, now_unix: u64) -> u32 {
        self.mutate(|book| {
            let Some(position) = book.clubs.iter().position(|club| {
                club.member_index(nickname)
                    .is_some_and(|index| club.members[index].grade == CLUB_GRADE_MASTER)
            }) else {
                return (CLUB_RESULT_REFRESH, false);
            };
            book.clubs.remove(position);
            book.record_departure(nickname, now_unix);
            (CLUB_RESULT_OK, true)
        })
    }

    pub(crate) fn set_intro(&self, nickname: &str, intro: &str) -> u32 {
        self.mutate(|book| match Self::club_of_mut(book, nickname) {
            Some(club) if club.can_manage(nickname) => {
                intro.clone_into(&mut club.intro);
                (CLUB_RESULT_OK, true)
            }
            _ => (CLUB_RESULT_REFRESH, false),
        })
    }

    pub(crate) fn set_auto_join(&self, nickname: &str, auto_join: bool) -> u32 {
        self.mutate(|book| match Self::club_of_mut(book, nickname) {
            Some(club) if club.can_manage(nickname) => {
                club.auto_join = auto_join;
                (CLUB_RESULT_OK, true)
            }
            _ => (CLUB_RESULT_REFRESH, false),
        })
    }

    pub(crate) fn rename(&self, nickname: &str, name: &str) -> (u32, u32) {
        let name = name.trim().to_owned();
        self.mutate(|book| {
            let duplicate = book
                .clubs
                .iter()
                .any(|club| club.name.to_lowercase() == name.to_lowercase());
            match Self::club_of_mut(book, nickname) {
                Some(club) if club.can_manage(nickname) && !name.is_empty() && !duplicate => {
                    club.name = name;
                    ((club.code, CLUB_RESULT_OK), true)
                }
                Some(club) => ((club.code, CLUB_RESULT_REFRESH), false),
                None => ((0, CLUB_RESULT_REFRESH), false),
            }
        })
    }

    pub(crate) fn set_mark(&self, nickname: &str, mark_logo: u32, mark_line: u32) -> (u32, u32) {
        self.mutate(|book| match Self::club_of_mut(book, nickname) {
            Some(club) if club.can_manage(nickname) => {
                club.mark_logo = mark_logo;
                club.mark_line = mark_line;
                ((club.code, CLUB_RESULT_OK), true)
            }
            Some(club) => ((club.code, CLUB_RESULT_REFRESH), false),
            None => ((0, CLUB_RESULT_REFRESH), false),
        })
    }

    pub(crate) fn kick(&self, nickname: &str, user_no: u32) -> u32 {
        self.mutate(|book| {
            let Some(club) = Self::club_of_mut(book, nickname) else {
                return (CLUB_RESULT_REFRESH, false);
            };
            if !club.can_manage(nickname) {
                return (CLUB_RESULT_REFRESH, false);
            }
            let Some(index) = club
                .members
                .iter()
                .position(|member| member.user_no == user_no)
            else {
                return (CLUB_RESULT_REFRESH, false);
            };
            if club.members[index].grade == CLUB_GRADE_MASTER {
                return (KICK_RESULT_MASTER, false);
            }
            club.members.remove(index);
            (CLUB_RESULT_OK, true)
        })
    }

    pub(crate) fn update_grade(&self, nickname: &str, user_no: u32, grade: u16) -> u32 {
        if !(CLUB_GRADE_MEMBER..=CLUB_GRADE_MANAGER).contains(&grade) {
            return CLUB_RESULT_REFRESH;
        }
        self.mutate(|book| {
            let Some(club) = Self::club_of_mut(book, nickname) else {
                return (CLUB_RESULT_REFRESH, false);
            };
            let is_master = club
                .member_index(nickname)
                .is_some_and(|index| club.members[index].grade == CLUB_GRADE_MASTER);
            match club
                .members
                .iter_mut()
                .find(|member| member.user_no == user_no)
            {
                Some(member) if is_master && member.grade != CLUB_GRADE_MASTER => {
                    member.grade = grade;
                    (CLUB_RESULT_OK, true)
                }
                _ => (CLUB_RESULT_REFRESH, false),
            }
        })
    }

    pub(crate) fn process_waiting(
        &self,
        nickname: &str,
        user_no: u32,
        accept: bool,
        now: LegacyTime,
    ) -> u32 {
        self.mutate(|book| {
            let Some(position) = book.clubs.iter().position(|club| club.can_manage(nickname))
            else {
                return (CLUB_RESULT_REFRESH, false);
            };
            let club = &mut book.clubs[position];
            let Some(index) = club
                .waiting
                .iter()
                .position(|waiting| waiting.user_no == user_no)
            else {
                return (CLUB_RESULT_REFRESH, false);
            };
            let waiting = club.waiting.remove(index);
            if accept && club.members.len() < MAX_CLUB_MEMBERS {
                club.members.push(StoredMember {
                    nickname: waiting.nickname.clone(),
                    user_no: waiting.user_no,
                    rp: waiting.rp,
                    grade: CLUB_GRADE_MEMBER,
                    joined_days: now.days_since_1900,
                    joined_quarter_seconds: now.quarter_seconds,
                });
                let joined_elsewhere = book.clubs.iter().enumerate().any(|(other, club)| {
                    other != position && club.member_index(&waiting.nickname).is_some()
                });
                if joined_elsewhere {
                    book.clubs[position]
                        .members
                        .retain(|member| !same_rider(&member.nickname, &waiting.nickname));
                }
            }
            (CLUB_RESULT_OK, true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> LegacyTime {
        time(45_000, 10)
    }

    fn actor(nickname: &str, user_no: u32) -> ClubActor<'_> {
        ClubActor {
            nickname,
            user_no,
            rp: 1_000,
        }
    }

    #[test]
    fn create_join_leave_and_disband_round_trip() {
        let clubs = ClubDirectory::in_memory();
        let (code, result) = clubs.create(actor("Master", 1), "Club", "Hello", 22, 0, now());
        assert_eq!((code, result), (FIRST_CLUB_CODE, CREATE_CLUB_SUCCESS));
        assert_eq!(
            clubs.create(actor("Other", 2), "club", "", 22, 0, now()).1,
            CREATE_CLUB_NAME_DUPLICATE
        );
        assert_eq!(
            clubs
                .create(actor("master", 1), "Second", "", 22, 0, now())
                .1,
            CREATE_CLUB_ALREADY_JOINED
        );

        let joined = clubs.join(actor("Crew", 3), code, now());
        assert_eq!(joined.result, JOIN_CLUB_JOINED);
        assert_eq!(joined.grade, CLUB_GRADE_MEMBER);
        assert_eq!(clubs.club_info(code).unwrap().member_count, 2);
        assert_eq!(clubs.search("cl", "", 0).len(), 1);
        assert_eq!(clubs.search("", "master", 0).len(), 1);
        assert_eq!(clubs.count("zzz", ""), 0);

        let membership = clubs.membership(actor("Crew", 30)).unwrap();
        assert_eq!(membership.me.user_no, 30);
        assert_eq!(membership.info.master_nickname, "Master");

        assert_eq!(clubs.leave("Master", 1_000), CLUB_RESULT_OK);
        assert!(clubs.leave_restricted("master", 1_000 + LEAVE_RESTRICTION_SECONDS - 1));
        assert!(!clubs.leave_restricted("Master", 1_000 + LEAVE_RESTRICTION_SECONDS));
        assert!(!clubs.leave_restricted("Crew", 1_000));
        let crew = clubs.membership(actor("Crew", 30)).unwrap();
        assert_eq!(crew.me.grade, CLUB_GRADE_MASTER);
        assert_eq!(clubs.break_up("Crew", 2_000), CLUB_RESULT_OK);
        assert!(clubs.leave_restricted("Crew", 2_000));
        assert!(clubs.club_info(code).is_none());
    }

    #[test]
    fn closed_clubs_queue_riders_until_a_manager_accepts() {
        let clubs = ClubDirectory::in_memory();
        let (code, _) = clubs.create(actor("Master", 1), "Club", "", 22, 0, now());
        assert_eq!(clubs.set_auto_join("Master", false), CLUB_RESULT_OK);
        assert_eq!(
            clubs.join(actor("Crew", 3), code, now()).result,
            JOIN_CLUB_WAITING
        );
        assert_eq!(clubs.waiting_count(code).0, 1);
        assert_eq!(
            clubs.waiting_club_of("crew"),
            Some((code, "Club".to_owned()))
        );
        assert_eq!(
            clubs.process_waiting("Master", 3, true, now()),
            CLUB_RESULT_OK
        );
        assert!(clubs.waiting_club_of("Crew").is_none());
        assert!(clubs.club_info_of("Crew").is_some());
        assert_eq!(clubs.kick("Crew", 1), CLUB_RESULT_REFRESH);
        assert_eq!(clubs.kick("Master", 1), KICK_RESULT_MASTER);
        assert_eq!(
            clubs.update_grade("Master", 3, CLUB_GRADE_MANAGER),
            CLUB_RESULT_OK
        );
        assert_eq!(clubs.kick("Master", 3), CLUB_RESULT_OK);
    }

    #[test]
    fn donations_raise_capital_and_rank_supporters() {
        let clubs = ClubDirectory::in_memory();
        let (code, _) = clubs.create(actor("Master", 1), "Club", "", 22, 0, now());
        clubs.join(actor("Crew", 2), code, now());
        let bank = clubs.bank("Crew").unwrap();
        assert_eq!((bank.capital, bank.my_total), (INITIAL_CLUB_CAPITAL, 0));
        assert_eq!(bank.top_supporter, "");

        clubs.support("Master", 10, now()).unwrap();
        let bank = clubs.support("crew", 50, now()).unwrap();
        assert_eq!(bank.capital, INITIAL_CLUB_CAPITAL + 60);
        assert_eq!(bank.my_total, 50);
        assert_eq!(bank.top_supporter, "Crew");
        assert_eq!(bank.history.len(), 2);
        assert_eq!(bank.history[0].nickname, "Crew");
        assert!(clubs.support("Outsider", 1, now()).is_none());
    }

    #[test]
    fn the_directory_survives_a_reopen() {
        let root = tempfile::tempdir().unwrap();
        let clubs = ClubDirectory::open(root.path());
        let (code, _) = clubs.create(actor("Master", 1), "Club", "Intro", 22, 3, now());
        clubs.join(actor("Crew", 2), code, now());
        assert_eq!(clubs.leave("Crew", 5_000), CLUB_RESULT_OK);
        drop(clubs);
        let reopened = ClubDirectory::open(root.path());
        let info = reopened.club_info(code).unwrap();
        assert_eq!(
            (info.name.as_str(), info.mark_logo, info.mark_line),
            ("Club", 22, 3)
        );
        assert!(reopened.leave_restricted("Crew", 5_001));
        let (next, _) = reopened.create(actor("Other", 2), "Second", "", 22, 0, now());
        assert_eq!(next, code + 1);
    }
}
