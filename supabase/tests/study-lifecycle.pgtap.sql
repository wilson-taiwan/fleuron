-- Study lifecycle pgTAP tests.
-- Verifies preflight RPC, last-member guard, sole-admin promotion,
-- idempotent project creation, and safe deletion cascades.

begin;

select no_plan();

-- ── 1. Permissions ───────────────────────────────────────────────────────────
select ok(
  has_function_privilege(
    'authenticated',
    'public.study_lifecycle_preflight(text)',
    'execute'
  ),
  'authenticated clients can call study_lifecycle_preflight'
);

select ok(
  not has_function_privilege(
    'anon',
    'public.study_lifecycle_preflight(text)',
    'execute'
  ),
  'anonymous clients cannot call study_lifecycle_preflight'
);

select ok(
  has_function_privilege(
    'authenticated',
    'public.create_project_idempotent(text,text,text,text)',
    'execute'
  ),
  'authenticated clients can call create_project_idempotent'
);

select ok(
  has_function_privilege(
    'authenticated',
    'public.leave_group(text)',
    'execute'
  ),
  'authenticated clients can call leave_group'
);

select ok(
  has_function_privilege(
    'authenticated',
    'public.delete_group(text,text)',
    'execute'
  ),
  'authenticated clients can call delete_group'
);

-- ── 2. Synthetic Test Setup ──────────────────────────────────────────────────
-- Setup mock users
insert into auth.users (id, email)
values
  ('11111111-1111-4111-8111-111111111111', 'alice@test.local'),
  ('22222222-2222-4222-8222-222222222222', 'bob@test.local')
on conflict (id) do nothing;

-- Create project with Alice as sole member/admin
insert into public.projects (project_id, title, group_key)
values ('lifecycle-test-1', 'Test Lifecycle Study', 'KEY11111')
on conflict (project_id) do nothing;

insert into public.project_members (project_id, user_id, coder_name, role)
values ('lifecycle-test-1', '11111111-1111-4111-8111-111111111111', 'Alice', 'admin')
on conflict (project_id, user_id) do update set role = 'admin';

-- Set auth context to Alice
select set_config('request.jwt.claims', '{"sub":"11111111-1111-4111-8111-111111111111","email":"alice@test.local"}', true);
set local role authenticated;

-- ── 3. Preflight RPC for sole member ─────────────────────────────────────────
select is(
  (public.study_lifecycle_preflight('lifecycle-test-1')->>'lifecycle_capability_version')::int,
  1,
  'preflight returns capability version 1'
);

select is(
  (public.study_lifecycle_preflight('lifecycle-test-1')->>'member_count')::int,
  1,
  'preflight counts 1 member'
);

select is(
  (public.study_lifecycle_preflight('lifecycle-test-1')->>'can_leave')::boolean,
  false,
  'sole member cannot leave'
);

select is(
  (public.study_lifecycle_preflight('lifecycle-test-1')->>'can_delete')::boolean,
  true,
  'admin can delete'
);

-- ── 4. Leave group as sole member is rejected ────────────────────────────────
select throws_ok(
  $$ select public.leave_group('lifecycle-test-1') $$,
  'P0001',
  'LAST_MEMBER_CANNOT_LEAVE',
  'sole member leaving is rejected by leave_group'
);

-- ── 5. Add second member (Bob) and test succession ───────────────────────────
reset role;
insert into public.project_members (project_id, user_id, coder_name, role, joined_at)
values ('lifecycle-test-1', '22222222-2222-4222-8222-222222222222', 'Bob', 'coder', now())
on conflict (project_id, user_id) do update set role = 'coder';

set local role authenticated;

select is(
  (public.study_lifecycle_preflight('lifecycle-test-1')->>'can_leave')::boolean,
  true,
  'with 2 members, Alice can leave'
);

select is(
  (public.study_lifecycle_preflight('lifecycle-test-1')->>'successor_coder_name'),
  'Bob',
  'Bob is identified as successor administrator'
);

-- Alice leaves; Bob should be auto-promoted to admin
select public.leave_group('lifecycle-test-1');

reset role;
select is(
  (select role from public.project_members where project_id = 'lifecycle-test-1' and user_id = '22222222-2222-4222-8222-222222222222'),
  'admin',
  'Bob is automatically promoted to admin after sole admin leaves'
);

-- ── 6. Direct delete on project_members cannot delete last member ────────────
select throws_ok(
  $$ delete from public.project_members where project_id = 'lifecycle-test-1' $$,
  'P0001',
  'LAST_MEMBER_CANNOT_LEAVE',
  'direct deletion of sole remaining member is rejected by trigger'
);

-- ── 7. Whole project cascade delete works ────────────────────────────────────
delete from public.projects where project_id = 'lifecycle-test-1';

select is(
  (select count(*) from public.project_members where project_id = 'lifecycle-test-1'),
  0::bigint,
  'whole project delete cascades and removes members cleanly'
);

-- ── 8. Idempotent Project Creation ───────────────────────────────────────────
select set_config('request.jwt.claims', '{"sub":"11111111-1111-4111-8111-111111111111","email":"alice@test.local"}', true);
set local role authenticated;

select is(
  (public.create_project_idempotent('req-123', 'idemp-proj-1', 'Idempotent Study', 'KEYIDEMP1')->>'idempotent_replay')::boolean,
  false,
  'first call creates project with idempotent_replay=false'
);

select is(
  (public.create_project_idempotent('req-123', 'idemp-proj-1', 'Idempotent Study', 'KEYIDEMP1')->>'idempotent_replay')::boolean,
  true,
  'replaying same request_id returns idempotent_replay=true'
);

select finish();
rollback;
