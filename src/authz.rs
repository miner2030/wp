use crate::auth::{Session, User};
use crate::db::{AccessRule, Share};

/// 目录访问级别:guest < login < admin。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Access {
    Guest,
    Login,
    Admin,
}

impl Access {
    pub fn as_str(&self) -> &'static str {
        match self {
            Access::Guest => "guest",
            Access::Login => "login",
            Access::Admin => "admin",
        }
    }

    pub fn from_str(s: &str) -> Option<Access> {
        match s {
            "guest" => Some(Access::Guest),
            "login" => Some(Access::Login),
            "admin" => Some(Access::Admin),
            _ => None,
        }
    }
}

fn rule_covers(rule_path: &str, rel: &str) -> bool {
    rule_path.is_empty() || rel == rule_path || rel.starts_with(&format!("{rule_path}/"))
}

/// 求某路径的访问级别:取前缀最深的覆盖规则,无规则时按 admin 处理。
pub fn access_of(rules: &[AccessRule], rel: &str) -> Access {
    rules
        .iter()
        .filter(|r| rule_covers(&r.rel_path, rel))
        .max_by_key(|r| (r.rel_path.matches('/').count(), r.rel_path.len()))
        .map(|r| r.access)
        .unwrap_or(Access::Admin)
}

/// 私有空间(有属主):仅属主与管理员可访问,忽略规则表。
pub fn private(share: &Share) -> bool {
    share.owner_id.is_some()
}

pub fn is_owner(share: &Share, u: &User) -> bool {
    u.is_admin || share.owner_id == Some(u.id)
}

pub fn check_read(session: &Session, share: &Share, rules: &[AccessRule], rel: &str) -> crate::error::ApiResult<()> {
    if private(share) {
        return match session.user() {
            Some(u) if is_owner(share, u) => Ok(()),
            _ => Err(crate::error::ApiError::forbidden("无权访问该空间")),
        };
    }
    match access_of(rules, rel) {
        Access::Guest => Ok(()),
        Access::Login => {
            if session.user.is_some() {
                Ok(())
            } else {
                Err(crate::error::ApiError::unauthorized("该目录需要登录后才能访问"))
            }
        }
        Access::Admin => {
            if session.is_admin() {
                Ok(())
            } else {
                Err(crate::error::ApiError::forbidden("该目录仅管理员可访问"))
            }
        }
    }
}

pub fn check_download(session: &Session, share: &Share, rules: &[AccessRule], rel: &str) -> crate::error::ApiResult<()> {
    check_read(session, share, rules, rel)?;
    if let Some(u) = session.user() {
        if !private(share) && !u.can_download {
            return Err(crate::error::ApiError::forbidden("当前账号无下载权限"));
        }
    }
    Ok(())
}

pub fn check_mutate(
    session: &Session,
    share: &Share,
    rules: &[AccessRule],
    rel: &str,
    flag: impl Fn(&User) -> bool,
) -> crate::error::ApiResult<()> {
    let u = require_login(session)?;
    if private(share) {
        if !is_owner(share, u) {
            return Err(crate::error::ApiError::forbidden("无权修改该空间"));
        }
    } else if access_of(rules, rel) == Access::Admin && !u.is_admin {
        return Err(crate::error::ApiError::forbidden("该目录仅管理员可访问"));
    }
    if flag(u) {
        Ok(())
    } else {
        Err(crate::error::ApiError::forbidden("当前账号无此操作权限"))
    }
}

pub fn can_read(session: &Session, share: &Share, rules: &[AccessRule], rel: &str) -> bool {
    if private(share) {
        return session.user().map(|u| is_owner(share, u)).unwrap_or(false);
    }
    match access_of(rules, rel) {
        Access::Guest => true,
        Access::Login => session.user.is_some(),
        Access::Admin => session.is_admin(),
    }
}

/// 下载权限:在可读基础上,登录用户还需开通下载开关;访客或私有点击不受限制。
pub fn can_download(session: &Session, share: &Share, rules: &[AccessRule], rel: &str) -> bool {
    if !can_read(session, share, rules, rel) {
        return false;
    }
    match session.user() {
        Some(u) => private(share) || u.can_download,
        None => true,
    }
}

/// 写操作(上传/建目录/重命名/删除):需登录、拥有该路径权限,且账号开关打开。
pub fn can_mutate(
    session: &Session,
    share: &Share,
    rules: &[AccessRule],
    rel: &str,
    flag: impl Fn(&User) -> bool,
) -> bool {
    let Some(u) = session.user() else { return false };
    let authorized = if private(share) {
        is_owner(share, u)
    } else if access_of(rules, rel) == Access::Admin {
        u.is_admin
    } else {
        true
    };
    authorized && flag(u)
}

pub fn require_login(session: &Session) -> crate::error::ApiResult<&User> {
    session.user().ok_or_else(|| crate::error::ApiError::unauthorized("需要登录"))
}

pub fn require_admin(session: &Session) -> crate::error::ApiResult<&User> {
    let u = require_login(session)?;
    if !u.is_admin {
        return Err(crate::error::ApiError::forbidden("需要管理员权限"));
    }
    Ok(u)
}