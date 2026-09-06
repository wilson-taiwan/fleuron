-- Study lifecycle guards: preflight capability, last-member protection,
-- request-ID idempotency, and serialized membership mutation.
--
-- Server schema certification stays 10: this migration is purely additive.

begin;

-- ── 1. Request-ID Idempotency for Shared Study Creation ──────────────────────
create table if not exists public.project_creation_requests (
  request_id text not null,
  user_id uuid not null references auth.users(id) on delete cascade,
  project_id text not null references public.projects(project_id) on delete cascade,
  group_key text not null,
  created_at timestamptz not null default now(),
  primary key (user_id, request_id)
);

alter table public.project_creation_requests enable row level security;

drop policy if exists pcr_owner_all on public.project_creation_requests;
create policy pcr_owner_all on public.project_creation_requests
  for all to authenticated
  using (user_id = auth.uid())
  with check (user_id = auth.uid());

grant select, insert, update, delete on table public.project_creation_requests to authenticated;

create or replace function public.create_project_idempotent(
  p_request_id text,
  p_project_id text,
  p_title text,
  p_group_key text
)
returns jsonb
language plpgsql security definer
set search_path = public
as $$
declare
  existing_record record;
begin
  if auth.uid() is null then
    raise exception 'Sign in first.';
  end if;

  perform public.require_sync_entitlement();

  if p_request_id is not null and btrim(p_request_id) <> '' then
    select r.project_id, r.group_key into existing_record
      from public.project_creation_requests r
     where r.user_id = auth.uid() and r.request_id = p_request_id;

    if found then
      return jsonb_build_object(
        'project_id', existing_record.project_id,
        'group_key', existing_record.group_key,
        'idempotent_replay', true
      );
    end if;
  end if;

  insert into public.projects (project_id, title, group_key)
  values (p_project_id, p_title, p_group_key);

  if p_request_id is not null and btrim(p_request_id) <> '' then
    insert into public.project_creation_requests (request_id, user_id, project_id, group_key)
    values (p_request_id, auth.uid(), p_project_id, p_group_key)
    on conflict (user_id, request_id) do nothing;
  end if;

  return jsonb_build_object(
    'project_id', p_project_id,
    'group_key', p_group_key,
    'idempotent_replay', false
  );
end;
$$;

revoke execute on function public.create_project_idempotent(text, text, text, text) from anon, public;
grant execute on function public.create_project_idempotent(text, text, text, text) to authenticated;

-- ── 2. Study Lifecycle Preflight RPC ──────────────────────────────────────────
create or replace function public.study_lifecycle_preflight(p_project_id text)
returns jsonb
language plpgsql security definer
set search_path = public
as $$
declare
  v_caller_role text;
  v_member_count int;
  v_successor_user_id uuid;
  v_successor_coder_name text;
  v_project_exists boolean;
begin
  if auth.uid() is null then
    raise exception 'Sign in first.';
  end if;

  select exists(select 1 from public.projects where project_id = p_project_id)
    into v_project_exists;

  if not v_project_exists then
    return jsonb_build_object(
      'lifecycle_capability_version', 1,
      'project_exists', false,
      'is_member', false,
      'role', null,
      'member_count', 0,
      'can_leave', false,
      'can_delete', false
    );
  end if;

  select m.role into v_caller_role
    from public.project_members m
   where m.project_id = p_project_id and m.user_id = auth.uid();

  if v_caller_role is null then
    return jsonb_build_object(
      'lifecycle_capability_version', 1,
      'project_exists', true,
      'is_member', false,
      'role', null,
      'member_count', 0,
      'can_leave', false,
      'can_delete', false
    );
  end if;

  select count(*) into v_member_count
    from public.project_members m
   where m.project_id = p_project_id;

  if v_caller_role = 'admin' and v_member_count > 1 then
    select m.user_id, m.coder_name
      into v_successor_user_id, v_successor_coder_name
      from public.project_members m
     where m.project_id = p_project_id
       and m.user_id <> auth.uid()
     order by m.joined_at asc, m.user_id asc
     limit 1;
  end if;

  return jsonb_build_object(
    'lifecycle_capability_version', 1,
    'project_exists', true,
    'is_member', true,
    'role', v_caller_role,
    'member_count', v_member_count,
    'can_leave', (v_member_count > 1),
    'can_delete', (v_caller_role = 'admin'),
    'successor_user_id', v_successor_user_id,
    'successor_coder_name', v_successor_coder_name
  );
end;
$$;

revoke execute on function public.study_lifecycle_preflight(text) from anon, public;
grant execute on function public.study_lifecycle_preflight(text) to authenticated;

-- ── 3. Guard against memberless studies on direct deletion ───────────────────
create or replace function public.check_project_members_not_empty()
returns trigger
language plpgsql security definer
set search_path = public
as $$
declare
  v_remaining int;
  v_project_exists boolean;
begin
  select exists(select 1 from public.projects where project_id = old.project_id)
    into v_project_exists;

  if v_project_exists then
    select count(*) into v_remaining
      from public.project_members
     where project_id = old.project_id
       and user_id <> old.user_id;

    if v_remaining = 0 then
      raise exception 'LAST_MEMBER_CANNOT_LEAVE'
        using errcode = 'P0001',
        detail = 'A study cannot be left with zero members while the project exists. Delete the project instead.';
    end if;
  end if;

  return old;
end;
$$;

drop trigger if exists trg_check_project_members_not_empty on public.project_members;
create trigger trg_check_project_members_not_empty
  before delete on public.project_members
  for each row
  execute function public.check_project_members_not_empty();

-- ── 4. Serialized, idempotent leave_group with last-member guard ─────────────
create or replace function public.leave_group(p_project_id text)
returns void
language plpgsql security definer
set search_path = public
as $$
declare
  caller_role text;
  total_members int;
  admin_count int;
  promote_uid uuid;
begin
  if auth.uid() is null then
    raise exception 'Sign in first.';
  end if;

  -- Lock project row to serialize membership operations
  perform 1 from public.projects where project_id = p_project_id for update;

  select m.role into caller_role
    from public.project_members m
   where m.project_id = p_project_id and m.user_id = auth.uid();

  -- Idempotent return if already absent
  if caller_role is null then
    return;
  end if;

  -- Block leave if sole member
  select count(*) into total_members
    from public.project_members m
   where m.project_id = p_project_id;

  if total_members <= 1 then
    raise exception 'LAST_MEMBER_CANNOT_LEAVE'
      using errcode = 'P0001',
      detail = 'Cannot leave study as the sole member.';
  end if;

  -- Promote successor if caller is sole admin
  if caller_role = 'admin' then
    select count(*) into admin_count
      from public.project_members m
     where m.project_id = p_project_id and m.role = 'admin';

    if admin_count <= 1 then
      select m.user_id into promote_uid
        from public.project_members m
       where m.project_id = p_project_id
         and m.user_id <> auth.uid()
       order by m.joined_at asc, m.user_id asc
       limit 1;

      if promote_uid is not null then
        update public.project_members
           set role = 'admin'
         where project_id = p_project_id and user_id = promote_uid;
      end if;
    end if;
  end if;

  delete from public.project_members
   where project_id = p_project_id and user_id = auth.uid();
end;
$$;

revoke execute on function public.leave_group(text) from anon, public;
grant execute on function public.leave_group(text) to authenticated;

-- ── 5. Serialized, idempotent delete_group ───────────────────────────────────
create or replace function public.delete_group(
  p_project_id text,
  p_confirm_title text
)
returns void
language plpgsql security definer
set search_path = public
as $$
declare
  caller_role text;
  current_title text;
begin
  if auth.uid() is null then
    raise exception 'Sign in first.';
  end if;

  -- Lock project row if exists
  perform 1 from public.projects where project_id = p_project_id for update;

  select p.title into current_title
    from public.projects p
   where p.project_id = p_project_id;

  if not found then
    -- Idempotent return on repeat/retry
    return;
  end if;

  select m.role into caller_role
    from public.project_members m
   where m.project_id = p_project_id and m.user_id = auth.uid();

  if caller_role is distinct from 'admin' then
    raise exception 'Only group administrators can delete a group.';
  end if;

  if btrim(p_confirm_title) <> btrim(current_title) then
    raise exception 'Confirmation title does not match group title.';
  end if;

  delete from public.projects
   where project_id = p_project_id;
end;
$$;

revoke execute on function public.delete_group(text, text) from anon, public;
grant execute on function public.delete_group(text, text) to authenticated;

notify pgrst, 'reload schema';

commit;
