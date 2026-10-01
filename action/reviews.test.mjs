import test from 'node:test';
import assert from 'node:assert/strict';
import { planReviews, reconcile, suggestion } from './reviews.mjs';

const login = 'github-actions[bot]';
const comment = (values = {}) => ({ path: 'a.ts', line: 2, text: '// increment', group: 'restates-code', decisionId: 'evidence-1', fix: { startLine: 2, endLine: 2, replacement: '' }, ...values });
const report = (comments = []) => ({ complete: true, stats: { unanswered: 0 }, comments });
const thread = (c = comment(), values = {}) => ({ id: 'thread-1', path: c.path, line: 2, originalLine: 2, isResolved: false, comments: { nodes: [{ databaseId: 42, author: { __typename: 'Bot', login }, body: suggestion(c).body, commit: { oid: 'old' } }] }, ...values });
const plan = (comments, threads, source = '') => planReviews(report(comments), threads, login, () => source);

test('resolves removed comments while retaining still-applicable threads', () => {
  assert.deepEqual(plan([], [thread()], 'count++;').resolve, ['thread-1']);
  assert.deepEqual(plan([comment()], [thread()]), { resolve: [], update: [], create: [] });
});
test('does not resolve score flips or scope exclusions with unchanged source', () => {
  assert.deepEqual(plan([comment({ group: null, fix: null })], [thread()]).resolve, []);
  assert.deepEqual(plan([], [thread()], '// increment\ncount++;').resolve, []);
});
test('resolves a retained comment rejudged after policy or evidence changes', () => {
  assert.deepEqual(plan([comment({ group: null, fix: null, decisionId: 'evidence-2' })], [thread()]).resolve, ['thread-1']);
});
test('never resolves a skipped or unanswered comment', () => {
  assert.deepEqual(plan([comment({ group: null, fix: null, decisionId: 'new', skipped: 'too-long' })], [thread()]).resolve, []);
  assert.throws(() => planReviews({ ...report(), complete: false }, [], login, () => ''), /incomplete/);
  assert.throws(() => planReviews({ ...report(), stats: { unanswered: 1 } }, [], login, () => ''), /incomplete/);
  assert.throws(() => planReviews({ comments: [], stats: { unanswered: 0 } }, [], login, () => ''), /incomplete/);
});
test('respects human resolutions and changed line numbers', () => {
  const c = comment({ line: 10, fix: { startLine: 10, endLine: 10, replacement: '' } });
  assert.deepEqual(plan([c], [thread(comment(), { line: 10, isResolved: true })]), { resolve: [], update: [], create: [] });
  assert.deepEqual(plan([c], [thread(comment(), { line: 10 })]).create, []);
});
test('never touches a human or another bot using the marker', () => {
  for (const author of [{ __typename: 'User', login }, { __typename: 'Bot', login: 'other[bot]' }]) {
    const t = thread(); t.comments.nodes[0].author = author;
    assert.deepEqual(plan([], [t]).resolve, []);
  }
});
test('does not collapse distinct occurrences of identical text', () => {
  const second = comment({ line: 8, fix: { startLine: 8, endLine: 8, replacement: '' } });
  const result = plan([comment(), second], [thread()]);
  assert.equal(result.create.length, 1); assert.equal(result.create[0].line, 8);
});
test('migrates legacy comments without deleting them', () => {
  const t = thread(); t.comments.nodes[0].body = suggestion(comment()).body.replace(/\n<!-- prolix:finding .*? -->/, '');
  assert.equal(plan([comment()], [t]).update.length, 1);
  assert.deepEqual(planReviews(report(), [t], login, (ref) => ref === 'head' ? 'count++;' : 'let count = 0;\n// increment\ncount++;').resolve, ['thread-1']);
  assert.deepEqual(plan([], [t], 'let count = 0;\n// increment\ncount++;').resolve, []);
});
test('missing files are resolved but read errors are not swallowed', () => {
  assert.deepEqual(plan([], [thread()], null).resolve, ['thread-1']);
  assert.throws(() => planReviews(report(), [thread()], login, () => { throw new Error('git failed'); }), /git failed/);
});
test('suggestions remain bounded and support multiline replacements', () => {
  assert.equal(plan(Array.from({ length: 51 }, (_, i) => comment({ path: `${i}.ts` })), []).create.length, 50);
  const s = suggestion(comment({ fix: { startLine: 1, endLine: 2, replacement: 'code();' } }));
  assert.equal(s.start_line, 1); assert.match(s.body, /```suggestion\ncode\(\);\n```/);
});
test('paginates threads and aborts stale runs before mutation', () => {
  const sha = 'a'.repeat(40), calls = [];
  const env = { GITHUB_REPOSITORY: 'owner/repo', PR: '1', HEAD_SHA: sha };
  const git = (args) => args.includes('--is-shallow-repository') ? 'false' : args[0] === 'rev-parse' ? sha : '';
  const api = (endpoint, body) => {
    calls.push(body);
    if (body.query.includes('reviewThreads')) return { data: { viewer: { login }, repository: { pullRequest: { headRefOid: sha, reviewThreads: { nodes: body.variables.cursor ? [thread()] : [], pageInfo: { hasNextPage: !body.variables.cursor, endCursor: 'next' } } } } } };
    return { data: { repository: { pullRequest: { headRefOid: 'b'.repeat(40) } } } };
  };
  assert.throws(() => reconcile(report(), env, api, git), /head changed/);
  assert.equal(calls.length, 3); assert.ok(calls.every((c) => !c.query.startsWith('mutation')));
});

test('recognizes GraphQL bot logins without the REST suffix', () => {
  const t = thread(); t.comments.nodes[0].author.login = 'github-actions';
  assert.deepEqual(plan([], [t]).resolve, ['thread-1']);
});
test('resolves via GraphQL and never deletes review comments', () => {
  const sha = 'a'.repeat(40), mutations = [];
  const env = { GITHUB_REPOSITORY: 'owner/repo', PR: '1', HEAD_SHA: sha };
  const git = (args) => args.includes('--is-shallow-repository') ? 'false' : args[0] === 'rev-parse' ? sha : '';
  const api = (endpoint, body, method) => {
    assert.notEqual(method, 'DELETE');
    if (body.query?.startsWith('mutation')) { mutations.push(body); return { data: { resolveReviewThread: { thread: { isResolved: true } } } }; }
    return { data: { viewer: { login }, repository: { pullRequest: { headRefOid: sha, reviewThreads: { nodes: [thread()], pageInfo: { hasNextPage: false } } } } } };
  };
  reconcile(report(), env, api, git);
  assert.equal(mutations.length, 1); assert.equal(mutations[0].variables.id, 'thread-1');
});

test('a materially changed finding can be raised after an earlier resolution', () => {
  assert.equal(plan([comment({ decisionId: 'changed-context' })], [thread(comment(), { isResolved: true })]).create.length, 1);
});
test('suggestions escape backtick fences in replacement code', () => {
  const body = suggestion(comment({ fix: { startLine: 1, endLine: 1, replacement: 'const text = `\n```\n`;' } })).body;
  assert.ok(body.includes('````suggestion\n'));
  assert.ok(body.endsWith('````'));
});

test('withdraws a previously offered automatic edit when it now needs review', () => {
  const result = plan([comment({ fix: null, fixStatus: 'below-fix-threshold' })], [thread()]);
  assert.equal(result.update.length, 1);
  assert.ok(!result.update[0].body.includes('```suggestion'));
  assert.deepEqual(result.resolve, []);
});

test('withdraws automatic edits after score flips without resolving the discussion', () => {
  const result = plan([comment({ group: null, fix: null })], [thread()]);
  assert.equal(result.update.length, 1);
  assert.ok(!result.update[0].body.includes('```suggestion'));
  assert.match(result.update[0].body, /left open for review/);
  assert.deepEqual(result.resolve, []);
});

test('withdraws automatic edits when the replacement no longer fits the anchor', () => {
  const result = plan([comment({ fix: { startLine: 2, endLine: 3, replacement: '' } })], [thread()]);
  assert.equal(result.update.length, 1);
  assert.ok(!result.update[0].body.includes('```suggestion'));
  assert.deepEqual(result.create, []);
});
