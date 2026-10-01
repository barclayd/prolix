import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const marker = '<!-- prolix:suggestion -->';
const botLogin = (login) => login.replace(/\[bot\]$/, '');
const normalize = (text) => text.split(/\r?\n/).map((line) => line.trim()).join('\n').trim();
const fingerprint = (comment) => createHash('sha256').update(comment.path + '\0' + normalize(comment.text)).digest('hex');

function metadata(body) {
  try {
    const encoded = body.match(/<!-- prolix:finding ([A-Za-z0-9_-]+) -->/)?.[1];
    const value = JSON.parse(Buffer.from(encoded ?? '', 'base64url').toString());
    return value.v === 1 && typeof value.text === 'string' ? value : null;
  } catch {
    return null;
  }
}

function reviewBody(comment) {
  const meta = { v: 1, id: fingerprint(comment), text: comment.text, decision: comment.decisionId };
  const prefix = `${marker}\n<!-- prolix:finding ${Buffer.from(JSON.stringify(meta)).toString('base64url')} -->\n**prolix**: ${comment.group ? `\`${comment.group}\`.` : 'The latest check did not flag this unchanged comment.'}`;
  if (!comment.group) return `${prefix} The earlier finding is left open for review; no automatic edit is available.`;
  if (!comment.fix) return `${prefix} This finding needs review; no automatic edit is available.`;
  const longestFence = Math.max(0, ...(comment.fix.replacement.match(/`+/g) ?? []).map((s) => s.length));
  const fence = '`'.repeat(Math.max(3, longestFence + 1));
  return `${prefix} Commit the suggestion to remove it.\n\n${fence}suggestion\n${comment.fix.replacement ? comment.fix.replacement + '\n' : ''}${fence}`;
}

export function suggestion(comment) {
  return {
    path: comment.path, line: comment.fix.endLine, side: 'RIGHT', body: reviewBody(comment),
    ...(comment.fix.startLine < comment.fix.endLine ? { start_line: comment.fix.startLine, start_side: 'RIGHT' } : {}),
  };
}

// readSource returns null only for a confirmed missing path; read failures must throw.
export function planReviews(report, threads, login, readSource) {
  if (report.complete !== true || report.stats?.unanswered !== 0) {
    throw new Error('incomplete report; review threads were left unchanged');
  }
  const comments = report.comments;
  const consumed = new Set();
  const plan = { resolve: [], update: [], create: [] };
  for (const thread of threads) {
    const first = thread.comments.nodes[0];
    if (first?.author?.__typename !== 'Bot' || botLogin(first.author.login) !== botLogin(login) || !first.body.startsWith(marker)) continue;
    const meta = metadata(first.body);
    let legacyText;
    if (!meta && first.commit?.oid && thread.originalLine) {
      const original = readSource(first.commit.oid, thread.path);
      if (original != null) legacyText = original.split(/\r?\n/).slice((thread.originalStartLine ?? thread.originalLine) - 1, thread.originalLine).join('\n');
    }
    const candidates = comments.filter((c) => !consumed.has(c) && c.path === thread.path && (
      meta ? fingerprint(c) === meta.id : (c.fix?.endLine ?? c.line + c.text.split('\n').length - 1) === thread.line && (legacyText ? normalize(legacyText).includes(normalize(c.text)) : reviewBody(c).split('\n').slice(2).join('\n') === first.body.split('\n').slice(1).join('\n'))
    ));
    candidates.sort((a, b) => Math.abs(a.line - (thread.line ?? thread.originalLine)) - Math.abs(b.line - (thread.line ?? thread.originalLine)));
    const current = candidates[0];
    if (current) {
      if (thread.isResolved && meta?.decision && current.decisionId && meta.decision !== current.decisionId) continue;
      consumed.add(current);
      if (thread.isResolved) continue;
      if (!current.group && meta?.decision && current.decisionId && meta.decision !== current.decisionId && !current.skipped) {
        plan.resolve.push(thread.id);
      } else {
        // An existing suggestion is valid only while its anchor and fix remain valid.
        const anchored = current.fix && thread.line === current.fix.endLine && (thread.startLine ?? thread.line) === current.fix.startLine;
        const body = reviewBody({ ...current, fix: anchored ? current.fix : null });
        if (body !== first.body) plan.update.push({ id: first.databaseId, body });
      }
      continue;
    }
    if (thread.isResolved) continue;
    const source = readSource('head', thread.path);
    const previous = meta?.text ?? legacyText;
    if (!previous) continue;
    // A score flip, changed scan scope, or newly protected directive is not evidence of a fix.
    if (normalize(previous) && (source === null || !normalize(source).includes(normalize(previous)))) plan.resolve.push(thread.id);
  }
  plan.create = comments.filter((c) => c.group && c.fix && !consumed.has(c)).slice(0, 50).map(suggestion);
  return plan;
}

const threadQuery = `query($owner:String!,$repo:String!,$pr:Int!,$cursor:String) {
  viewer { login }
  repository(owner:$owner,name:$repo) { pullRequest(number:$pr) {
    headRefOid reviewThreads(first:100,after:$cursor) {
      nodes { id isResolved isOutdated path line startLine originalLine originalStartLine
        comments(first:1) { nodes { databaseId body author { __typename login } commit { oid } } }
      } pageInfo { hasNextPage endCursor }
    }
  } }
}`;

export function reconcile(report, env, api, git, warn = console.warn) {
  const [owner, repo] = env.GITHUB_REPOSITORY.split('/');
  const pr = Number(env.PR), sha = env.HEAD_SHA;
  if (!owner || !repo || !Number.isInteger(pr) || pr < 1 || !/^[a-f0-9]{40,64}$/.test(sha)) throw new Error('invalid PR context');
  if (git(['rev-parse', 'HEAD']).trim() !== sha || git(['rev-parse', '--is-shallow-repository']).trim() !== 'false') {
    throw new Error('suggestions need the PR head checkout with fetch-depth: 0');
  }
  const variables = { owner, repo, pr };
  const threads = [];
  let cursor = null, login;
  do {
    const data = api('graphql', { query: threadQuery, variables: { ...variables, cursor } }).data;
    const pull = data.repository.pullRequest;
    if (pull.headRefOid !== sha) throw new Error('PR head changed; this run is stale');
    login = data.viewer.login;
    threads.push(...pull.reviewThreads.nodes);
    cursor = pull.reviewThreads.pageInfo.hasNextPage ? pull.reviewThreads.pageInfo.endCursor : null;
  } while (cursor);
  const sourceCache = new Map();
  const readSource = (ref, path) => {
    const revision = ref === 'head' ? sha : ref;
    const key = revision + ':' + path;
    if (!sourceCache.has(key)) {
      const entry = git(['--literal-pathspecs', 'ls-tree', '-z', revision, '--', path]);
      sourceCache.set(key, entry ? git(['show', key]) : null);
    }
    return sourceCache.get(key);
  };
  const plan = planReviews(report, threads, login, readSource);
  const current = () => {
    const data = api('graphql', { query: 'query($owner:String!,$repo:String!,$pr:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$pr){headRefOid}}}', variables }).data;
    if (data.repository.pullRequest.headRefOid !== sha) throw new Error('PR head changed; stopped updating review threads');
  };
  const prefix = `repos/${env.GITHUB_REPOSITORY}/pulls`;
  for (const { id, body } of plan.update) {
    current();
    api(`${prefix}/comments/${id}`, { body }, 'PATCH');
  }
  for (const comment of plan.create) {
    current();
    try { api(`${prefix}/${pr}/comments`, { ...comment, commit_id: sha }); }
    catch (error) { warn(`Could not post suggestion at ${comment.path}:${comment.line}: ${error.message}`); }
  }
  for (const id of plan.resolve) {
    current();
    api('graphql', { query: 'mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{isResolved}}}', variables: { id } });
  }
  return plan;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const api = (endpoint, body, method = 'POST') => {
    const data = JSON.parse(execFileSync('gh', ['api', endpoint, '--method', method, '--input', '-'], { input: JSON.stringify(body), encoding: 'utf8' }));
    if (data.errors?.length) throw new Error(data.errors.map((e) => e.message).join('; '));
    return data;
  };
  try {
    const report = JSON.parse(readFileSync(process.argv[2], 'utf8'));
    const plan = reconcile(report, process.env, api, (args) => execFileSync('git', args, { encoding: 'utf8' }));
    console.log(`prolix: ${plan.resolve.length} threads resolved, ${plan.create.length} suggestions attempted`);
  } catch (error) {
    console.error(`prolix: ${error.message}`);
    process.exitCode = 1;
  }
}
