# main.py
#
# Run:
#   pip install fastapi uvicorn sqlalchemy pydantic[email]
#   uvicorn main:app --reload
#
# Docs:
#   http://localhost:8000/docs

from __future__ import annotations

from datetime import datetime, timezone
from enum import Enum
from typing import Optional

from fastapi import Depends, FastAPI, HTTPException, Query, status
from pydantic import BaseModel, ConfigDict, EmailStr, Field
from sqlalchemy import (
    Boolean,
    DateTime,
    Enum as SQLEnum,
    ForeignKey,
    String,
    Text,
    create_engine,
    func,
    select,
)
from sqlalchemy.orm import (
    DeclarativeBase,
    Mapped,
    Session,
    mapped_column,
    relationship,
    sessionmaker,
)


# ============================================================
# Database
# ============================================================


DATABASE_URL = "sqlite:///./demo.db"


class Base(DeclarativeBase):
    pass


engine = create_engine(
    DATABASE_URL,
    connect_args={"check_same_thread": False},
)

SessionLocal = sessionmaker(
    bind=engine,
    autoflush=False,
    autocommit=False,
)


def get_db():
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


# ============================================================
# Enums
# ============================================================


class UserRole(str, Enum):
    admin = "admin"
    member = "member"


class ProjectStatus(str, Enum):
    draft = "draft"
    active = "active"
    archived = "archived"


class TaskStatus(str, Enum):
    todo = "todo"
    in_progress = "in_progress"
    done = "done"


class TaskPriority(str, Enum):
    low = "low"
    medium = "medium"
    high = "high"
    critical = "critical"


# ============================================================
# SQLAlchemy Models
# ============================================================


class User(Base):
    __tablename__ = "users"

    id: Mapped[int] = mapped_column(primary_key=True)

    name: Mapped[str] = mapped_column(String(120), index=True)
    email: Mapped[str] = mapped_column(
        String(255),
        unique=True,
        index=True,
    )

    role: Mapped[UserRole] = mapped_column(
        SQLEnum(UserRole),
        default=UserRole.member,
    )

    is_active: Mapped[bool] = mapped_column(
        Boolean,
        default=True,
    )

    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utcnow,
    )

    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utcnow,
        onupdate=utcnow,
    )

    projects: Mapped[list["Project"]] = relationship(
        back_populates="owner",
        cascade="all, delete-orphan",
    )

    assigned_tasks: Mapped[list["Task"]] = relationship(
        back_populates="assignee",
        foreign_keys="Task.assignee_id",
    )


class Project(Base):
    __tablename__ = "projects"

    id: Mapped[int] = mapped_column(primary_key=True)

    name: Mapped[str] = mapped_column(
        String(200),
        index=True,
    )

    description: Mapped[Optional[str]] = mapped_column(
        Text,
        nullable=True,
    )

    status: Mapped[ProjectStatus] = mapped_column(
        SQLEnum(ProjectStatus),
        default=ProjectStatus.draft,
        index=True,
    )

    owner_id: Mapped[int] = mapped_column(
        ForeignKey("users.id"),
        index=True,
    )

    is_deleted: Mapped[bool] = mapped_column(
        Boolean,
        default=False,
        index=True,
    )

    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utcnow,
    )

    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utcnow,
        onupdate=utcnow,
    )

    owner: Mapped["User"] = relationship(
        back_populates="projects",
    )

    tasks: Mapped[list["Task"]] = relationship(
        back_populates="project",
        cascade="all, delete-orphan",
    )


class Task(Base):
    __tablename__ = "tasks"

    id: Mapped[int] = mapped_column(primary_key=True)

    title: Mapped[str] = mapped_column(
        String(200),
        index=True,
    )

    description: Mapped[Optional[str]] = mapped_column(
        Text,
        nullable=True,
    )

    status: Mapped[TaskStatus] = mapped_column(
        SQLEnum(TaskStatus),
        default=TaskStatus.todo,
        index=True,
    )

    priority: Mapped[TaskPriority] = mapped_column(
        SQLEnum(TaskPriority),
        default=TaskPriority.medium,
        index=True,
    )

    project_id: Mapped[int] = mapped_column(
        ForeignKey("projects.id"),
        index=True,
    )

    assignee_id: Mapped[Optional[int]] = mapped_column(
        ForeignKey("users.id"),
        nullable=True,
        index=True,
    )

    due_at: Mapped[Optional[datetime]] = mapped_column(
        DateTime(timezone=True),
        nullable=True,
    )

    is_deleted: Mapped[bool] = mapped_column(
        Boolean,
        default=False,
        index=True,
    )

    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utcnow,
    )

    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utcnow,
        onupdate=utcnow,
    )

    project: Mapped["Project"] = relationship(
        back_populates="tasks",
    )

    assignee: Mapped[Optional["User"]] = relationship(
        back_populates="assigned_tasks",
        foreign_keys=[assignee_id],
    )


Base.metadata.create_all(bind=engine)


# ============================================================
# Pydantic Schemas
# ============================================================


class ORMModel(BaseModel):
    model_config = ConfigDict(from_attributes=True)


# -------------------------
# User Schemas
# -------------------------


class UserCreate(BaseModel):
    name: str = Field(min_length=2, max_length=120)
    email: EmailStr
    role: UserRole = UserRole.member


class UserUpdate(BaseModel):
    name: Optional[str] = Field(
        default=None,
        min_length=2,
        max_length=120,
    )

    email: Optional[EmailStr] = None
    role: Optional[UserRole] = None
    is_active: Optional[bool] = None


class UserRead(ORMModel):
    id: int
    name: str
    email: EmailStr
    role: UserRole
    is_active: bool
    created_at: datetime
    updated_at: datetime


# -------------------------
# Project Schemas
# -------------------------


class ProjectCreate(BaseModel):
    name: str = Field(min_length=2, max_length=200)
    description: Optional[str] = Field(
        default=None,
        max_length=2000,
    )

    owner_id: int

    status: ProjectStatus = ProjectStatus.draft


class ProjectUpdate(BaseModel):
    name: Optional[str] = Field(
        default=None,
        min_length=2,
        max_length=200,
    )

    description: Optional[str] = Field(
        default=None,
        max_length=2000,
    )

    owner_id: Optional[int] = None
    status: Optional[ProjectStatus] = None


class ProjectRead(ORMModel):
    id: int
    name: str
    description: Optional[str]
    status: ProjectStatus

    owner_id: int

    created_at: datetime
    updated_at: datetime


# -------------------------
# Task Schemas
# -------------------------


class TaskCreate(BaseModel):
    title: str = Field(min_length=1, max_length=200)

    description: Optional[str] = Field(
        default=None,
        max_length=5000,
    )

    project_id: int
    assignee_id: Optional[int] = None

    status: TaskStatus = TaskStatus.todo
    priority: TaskPriority = TaskPriority.medium

    due_at: Optional[datetime] = None


class TaskUpdate(BaseModel):
    title: Optional[str] = Field(
        default=None,
        min_length=1,
        max_length=200,
    )

    description: Optional[str] = Field(
        default=None,
        max_length=5000,
    )

    assignee_id: Optional[int] = None

    status: Optional[TaskStatus] = None
    priority: Optional[TaskPriority] = None

    due_at: Optional[datetime] = None


class TaskRead(ORMModel):
    id: int
    title: str
    description: Optional[str]

    project_id: int
    assignee_id: Optional[int]

    status: TaskStatus
    priority: TaskPriority

    due_at: Optional[datetime]

    created_at: datetime
    updated_at: datetime


# -------------------------
# Nested Response Schemas
# -------------------------


class TaskWithAssignee(TaskRead):
    assignee: Optional[UserRead] = None


class ProjectDetail(ProjectRead):
    owner: UserRead
    tasks: list[TaskWithAssignee]


class UserDetail(UserRead):
    projects: list[ProjectRead]


class PaginationMeta(BaseModel):
    page: int
    page_size: int
    total: int
    pages: int


class ProjectListResponse(BaseModel):
    items: list[ProjectRead]
    meta: PaginationMeta


class TaskListResponse(BaseModel):
    items: list[TaskWithAssignee]
    meta: PaginationMeta


# ============================================================
# FastAPI App
# ============================================================


app = FastAPI(
    title="FastAPI CRUD Demo",
    version="1.0.0",
    description="""
A medium-complexity CRUD demo containing:

- Users
- Projects
- Tasks
- Relationships
- Nested responses
- Search
- Filtering
- Pagination
- Soft deletion
- Enums
- Validation
""",
)


# ============================================================
# Helpers
# ============================================================


def get_user_or_404(
    db: Session,
    user_id: int,
) -> User:
    user = db.get(User, user_id)

    if not user:
        raise HTTPException(
            status_code=404,
            detail="User not found",
        )

    return user


def get_project_or_404(
    db: Session,
    project_id: int,
) -> Project:
    project = db.scalar(
        select(Project).where(
            Project.id == project_id,
            Project.is_deleted.is_(False),
        )
    )

    if not project:
        raise HTTPException(
            status_code=404,
            detail="Project not found",
        )

    return project


def get_task_or_404(
    db: Session,
    task_id: int,
) -> Task:
    task = db.scalar(
        select(Task).where(
            Task.id == task_id,
            Task.is_deleted.is_(False),
        )
    )

    if not task:
        raise HTTPException(
            status_code=404,
            detail="Task not found",
        )

    return task


# ============================================================
# Health
# ============================================================


@app.get("/")
def root():
    return {
        "name": "FastAPI CRUD Demo",
        "status": "ok",
        "docs": "/docs",
    }


@app.get("/health")
def health():
    return {
        "status": "healthy",
        "timestamp": utcnow(),
    }


# ============================================================
# Users CRUD
# ============================================================


@app.post(
    "/users",
    response_model=UserRead,
    status_code=status.HTTP_201_CREATED,
)
def create_user(
    data: UserCreate,
    db: Session = Depends(get_db),
):
    existing = db.scalar(
        select(User).where(
            User.email == str(data.email)
        )
    )

    if existing:
        raise HTTPException(
            status_code=409,
            detail="Email already exists",
        )

    user = User(
        name=data.name,
        email=str(data.email),
        role=data.role,
    )

    db.add(user)
    db.commit()
    db.refresh(user)

    return user


@app.get(
    "/users",
    response_model=list[UserRead],
)
def list_users(
    db: Session = Depends(get_db),
    search: Optional[str] = None,
    role: Optional[UserRole] = None,
    is_active: Optional[bool] = None,
    limit: int = Query(20, ge=1, le=100),
    offset: int = Query(0, ge=0),
):
    stmt = select(User)

    if search:
        search_value = f"%{search}%"

        stmt = stmt.where(
            User.name.ilike(search_value)
            | User.email.ilike(search_value)
        )

    if role:
        stmt = stmt.where(User.role == role)

    if is_active is not None:
        stmt = stmt.where(
            User.is_active == is_active
        )

    stmt = (
        stmt.order_by(User.created_at.desc())
        .offset(offset)
        .limit(limit)
    )

    return list(db.scalars(stmt).all())


@app.get(
    "/users/{user_id}",
    response_model=UserDetail,
)
def get_user(
    user_id: int,
    db: Session = Depends(get_db),
):
    return get_user_or_404(db, user_id)


@app.patch(
    "/users/{user_id}",
    response_model=UserRead,
)
def update_user(
    user_id: int,
    data: UserUpdate,
    db: Session = Depends(get_db),
):
    user = get_user_or_404(db, user_id)

    update_data = data.model_dump(
        exclude_unset=True
    )

    if "email" in update_data:
        email = str(update_data["email"])

        existing = db.scalar(
            select(User).where(
                User.email == email,
                User.id != user_id,
            )
        )

        if existing:
            raise HTTPException(
                status_code=409,
                detail="Email already exists",
            )

        update_data["email"] = email

    for field, value in update_data.items():
        setattr(user, field, value)

    user.updated_at = utcnow()

    db.commit()
    db.refresh(user)

    return user


@app.delete(
    "/users/{user_id}",
    status_code=status.HTTP_204_NO_CONTENT,
)
def delete_user(
    user_id: int,
    db: Session = Depends(get_db),
):
    user = get_user_or_404(db, user_id)

    owned_projects = db.scalar(
        select(func.count(Project.id)).where(
            Project.owner_id == user.id,
            Project.is_deleted.is_(False),
        )
    )

    if owned_projects:
        raise HTTPException(
            status_code=409,
            detail=(
                "Cannot delete user while they "
                "own active projects"
            ),
        )

    db.delete(user)
    db.commit()


# ============================================================
# Projects CRUD
# ============================================================


@app.post(
    "/projects",
    response_model=ProjectRead,
    status_code=status.HTTP_201_CREATED,
)
def create_project(
    data: ProjectCreate,
    db: Session = Depends(get_db),
):
    owner = get_user_or_404(
        db,
        data.owner_id,
    )

    if not owner.is_active:
        raise HTTPException(
            status_code=400,
            detail="Inactive user cannot own a project",
        )

    project = Project(
        name=data.name,
        description=data.description,
        owner_id=data.owner_id,
        status=data.status,
    )

    db.add(project)
    db.commit()
    db.refresh(project)

    return project


@app.get(
    "/projects",
    response_model=ProjectListResponse,
)
def list_projects(
    db: Session = Depends(get_db),
    search: Optional[str] = None,
    owner_id: Optional[int] = None,
    project_status: Optional[ProjectStatus] = Query(
        default=None,
        alias="status",
    ),
    page: int = Query(1, ge=1),
    page_size: int = Query(20, ge=1, le=100),
):
    filters = [
        Project.is_deleted.is_(False)
    ]

    if search:
        filters.append(
            Project.name.ilike(
                f"%{search}%"
            )
        )

    if owner_id is not None:
        filters.append(
            Project.owner_id == owner_id
        )

    if project_status:
        filters.append(
            Project.status == project_status
        )

    total = db.scalar(
        select(func.count(Project.id))
        .where(*filters)
    ) or 0

    offset = (page - 1) * page_size

    projects = list(
        db.scalars(
            select(Project)
            .where(*filters)
            .order_by(
                Project.created_at.desc()
            )
            .offset(offset)
            .limit(page_size)
        ).all()
    )

    pages = (
        (total + page_size - 1)
        // page_size
    )

    return {
        "items": projects,
        "meta": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "pages": pages,
        },
    }


@app.get(
    "/projects/{project_id}",
    response_model=ProjectDetail,
)
def get_project(
    project_id: int,
    db: Session = Depends(get_db),
):
    project = get_project_or_404(
        db,
        project_id,
    )

    # hide soft-deleted tasks
    project.tasks = [
        task
        for task in project.tasks
        if not task.is_deleted
    ]

    return project


@app.patch(
    "/projects/{project_id}",
    response_model=ProjectRead,
)
def update_project(
    project_id: int,
    data: ProjectUpdate,
    db: Session = Depends(get_db),
):
    project = get_project_or_404(
        db,
        project_id,
    )

    update_data = data.model_dump(
        exclude_unset=True
    )

    if "owner_id" in update_data:
        owner = get_user_or_404(
            db,
            update_data["owner_id"],
        )

        if not owner.is_active:
            raise HTTPException(
                status_code=400,
                detail=(
                    "Inactive user cannot own "
                    "a project"
                ),
            )

    for field, value in update_data.items():
        setattr(project, field, value)

    project.updated_at = utcnow()

    db.commit()
    db.refresh(project)

    return project


@app.delete(
    "/projects/{project_id}",
    status_code=status.HTTP_204_NO_CONTENT,
)
def delete_project(
    project_id: int,
    db: Session = Depends(get_db),
):
    project = get_project_or_404(
        db,
        project_id,
    )

    project.is_deleted = True
    project.updated_at = utcnow()

    # cascade soft-delete tasks
    for task in project.tasks:
        task.is_deleted = True
        task.updated_at = utcnow()

    db.commit()


# ============================================================
# Tasks CRUD
# ============================================================


@app.post(
    "/tasks",
    response_model=TaskRead,
    status_code=status.HTTP_201_CREATED,
)
def create_task(
    data: TaskCreate,
    db: Session = Depends(get_db),
):
    get_project_or_404(
        db,
        data.project_id,
    )

    if data.assignee_id is not None:
        assignee = get_user_or_404(
            db,
            data.assignee_id,
        )

        if not assignee.is_active:
            raise HTTPException(
                status_code=400,
                detail=(
                    "Cannot assign task "
                    "to inactive user"
                ),
            )

    task = Task(
        title=data.title,
        description=data.description,
        project_id=data.project_id,
        assignee_id=data.assignee_id,
        status=data.status,
        priority=data.priority,
        due_at=data.due_at,
    )

    db.add(task)
    db.commit()
    db.refresh(task)

    return task


@app.get(
    "/tasks",
    response_model=TaskListResponse,
)
def list_tasks(
    db: Session = Depends(get_db),
    search: Optional[str] = None,
    project_id: Optional[int] = None,
    assignee_id: Optional[int] = None,
    task_status: Optional[TaskStatus] = Query(
        default=None,
        alias="status",
    ),
    priority: Optional[TaskPriority] = None,
    overdue: Optional[bool] = None,
    page: int = Query(1, ge=1),
    page_size: int = Query(20, ge=1, le=100),
):
    filters = [
        Task.is_deleted.is_(False)
    ]

    if search:
        filters.append(
            Task.title.ilike(
                f"%{search}%"
            )
        )

    if project_id is not None:
        filters.append(
            Task.project_id == project_id
        )

    if assignee_id is not None:
        filters.append(
            Task.assignee_id == assignee_id
        )

    if task_status:
        filters.append(
            Task.status == task_status
        )

    if priority:
        filters.append(
            Task.priority == priority
        )

    if overdue is True:
        filters.extend(
            [
                Task.due_at.is_not(None),
                Task.due_at < utcnow(),
                Task.status != TaskStatus.done,
            ]
        )

    total = db.scalar(
        select(func.count(Task.id))
        .where(*filters)
    ) or 0

    offset = (page - 1) * page_size

    tasks = list(
        db.scalars(
            select(Task)
            .where(*filters)
            .order_by(
                Task.created_at.desc()
            )
            .offset(offset)
            .limit(page_size)
        ).all()
    )

    pages = (
        (total + page_size - 1)
        // page_size
    )

    return {
        "items": tasks,
        "meta": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "pages": pages,
        },
    }


@app.get(
    "/tasks/{task_id}",
    response_model=TaskWithAssignee,
)
def get_task(
    task_id: int,
    db: Session = Depends(get_db),
):
    return get_task_or_404(
        db,
        task_id,
    )


@app.patch(
    "/tasks/{task_id}",
    response_model=TaskRead,
)
def update_task(
    task_id: int,
    data: TaskUpdate,
    db: Session = Depends(get_db),
):
    task = get_task_or_404(
        db,
        task_id,
    )

    update_data = data.model_dump(
        exclude_unset=True
    )

    if "assignee_id" in update_data:
        assignee_id = update_data[
            "assignee_id"
        ]

        if assignee_id is not None:
            assignee = get_user_or_404(
                db,
                assignee_id,
            )

            if not assignee.is_active:
                raise HTTPException(
                    status_code=400,
                    detail=(
                        "Cannot assign task "
                        "to inactive user"
                    ),
                )

    for field, value in update_data.items():
        setattr(task, field, value)

    task.updated_at = utcnow()

    db.commit()
    db.refresh(task)

    return task


@app.delete(
    "/tasks/{task_id}",
    status_code=status.HTTP_204_NO_CONTENT,
)
def delete_task(
    task_id: int,
    db: Session = Depends(get_db),
):
    task = get_task_or_404(
        db,
        task_id,
    )

    task.is_deleted = True
    task.updated_at = utcnow()

    db.commit()


# ============================================================
# Additional Domain Operations
# ============================================================


@app.post(
    "/tasks/{task_id}/complete",
    response_model=TaskRead,
)
def complete_task(
    task_id: int,
    db: Session = Depends(get_db),
):
    task = get_task_or_404(
        db,
        task_id,
    )

    task.status = TaskStatus.done
    task.updated_at = utcnow()

    db.commit()
    db.refresh(task)

    return task


@app.post(
    "/projects/{project_id}/archive",
    response_model=ProjectRead,
)
def archive_project(
    project_id: int,
    db: Session = Depends(get_db),
):
    project = get_project_or_404(
        db,
        project_id,
    )

    incomplete_tasks = db.scalar(
        select(func.count(Task.id)).where(
            Task.project_id == project.id,
            Task.is_deleted.is_(False),
            Task.status != TaskStatus.done,
        )
    )

    if incomplete_tasks:
        raise HTTPException(
            status_code=409,
            detail=(
                f"Project has {incomplete_tasks} "
                "incomplete task(s)"
            ),
        )

    project.status = ProjectStatus.archived
    project.updated_at = utcnow()

    db.commit()
    db.refresh(project)

    return project


@app.get("/stats")
def stats(
    db: Session = Depends(get_db),
):
    users = db.scalar(
        select(func.count(User.id))
    ) or 0

    projects = db.scalar(
        select(func.count(Project.id)).where(
            Project.is_deleted.is_(False)
        )
    ) or 0

    tasks = db.scalar(
        select(func.count(Task.id)).where(
            Task.is_deleted.is_(False)
        )
    ) or 0

    completed_tasks = db.scalar(
        select(func.count(Task.id)).where(
            Task.is_deleted.is_(False),
            Task.status == TaskStatus.done,
        )
    ) or 0

    overdue_tasks = db.scalar(
        select(func.count(Task.id)).where(
            Task.is_deleted.is_(False),
            Task.status != TaskStatus.done,
            Task.due_at.is_not(None),
            Task.due_at < utcnow(),
        )
    ) or 0

    return {
        "users": users,
        "projects": projects,
        "tasks": {
            "total": tasks,
            "completed": completed_tasks,
            "overdue": overdue_tasks,
        },
    }
