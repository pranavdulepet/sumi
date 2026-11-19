import {test, expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createInterface} from 'node:readline';
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
async function fixtureNotebook(
  agents = false,
  datedHistory?: {markdown: string; scores: object[]},
  existingFolder?: string,
) {
  const folder = existingFolder ?? (await mkdtemp(join(tmpdir(), 'sumi-ui-')));
  if (datedHistory) {
    await mkdir(join(folder, 'scroll'), {recursive: true});
    await writeFile(join(folder, 'scroll/deciding.md'), datedHistory.markdown);
    await writeFile(
      join(folder, 'predictions.json'),
      JSON.stringify(datedHistory.scores),
    );
  }
  const core = spawn(
    'src-tauri/target/debug/sumi',
    [
      '--fixture-rpc',
      '--notebook',
      folder,
      ...(agents ? ['--fixture-agents'] : []),
      ...(datedHistory ? ['--fixture-predictions'] : []),
    ],
    {stdio: ['pipe', 'pipe', 'inherit']},
  );
  const pending: ((s: string) => void)[] = [];
  createInterface({input: core.stdout}).on('line', s => pending.shift()?.(s));
  function rpc(op: string, args = {}) {
    return new Promise<any>((resolve, reject) => {
      pending.push(s => {
        const result = JSON.parse(s);
        if (result.error) reject(Error(result.error));
        else resolve(result.ok);
      });
      core.stdin.write(JSON.stringify({operation: op, args}) + '\n');
    });
  }
  return {core, rpc, folder};
}

test('brushstroke, real local prediction, correction, history and persisted sheets', async ({
  page,
}, info) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(page.getByText('A mark of your own.')).toBeVisible();
    const box = await page
      .getByRole('img', {name: /Draw your personal/})
      .boundingBox();
    if (!box) throw Error('Missing drawing surface');
    await page.mouse.move(box.x + 35, box.y + 70);
    await page.mouse.down();
    await page.mouse.move(box.x + 180, box.y + 90, {steps: 16});
    await page.mouse.up();
    await page.getByRole('button', {name: 'Keep my mark'}).click();
    await page.getByRole('button', {name: /llama3.2:3b/}).click();
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await page
      .getByRole('button', {name: 'Continue with a clean page'})
      .click();
    await page.getByRole('button', {name: 'Open the Desk'}).click();
    await page
      .getByLabel('Desk sheet')
      .fill('Should I keep the meeting brief or make it detailed');
    await page.getByRole('button', {name: 'Seal a guess'}).click();
    await page.getByRole('button', {name: 'Open sealed guess'}).click();
    await page.getByLabel('Your decision').fill('Keep it brief.');
    await page.getByRole('button', {name: 'Break the seal'}).click();
    await expect(page.getByText('You decided · Keep it brief.')).toBeVisible();
    await page.getByRole('button', {name: 'It missed'}).click();
    await page.keyboard.press('Meta+3');
    await expect(
      page.getByText('0 of 1 scored guesses matched.'),
    ).toBeVisible();
    await page.keyboard.press('Meta+2');
    await page.getByRole('button', {name: 'Write an entry'}).click();
    await page
      .getByLabel('Your sentence')
      .fill('I choose short meetings when the agenda is clear.');
    await page.getByRole('button', {name: 'Keep in the Scroll'}).click();
    await page
      .getByRole('button', {
        name: 'I choose short meetings when the agenda is clear.',
      })
      .click();
    await page.getByRole('button', {name: 'Edit sentence'}).click();
    await page
      .getByLabel('Correct this entry')
      .fill('I prefer an agenda before accepting a meeting.');
    await page.getByRole('button', {name: 'Save correction'}).click();
    await expect(
      page.getByRole('button', {
        name: 'I prefer an agenda before accepting a meeting.',
      }),
    ).toBeVisible();
    const range = page.getByLabel('Scroll history');
    await range.focus();
    await range.press('Home');
    await expect(
      page.getByRole('button', {
        name: 'I choose short meetings when the agenda is clear.',
      }),
    ).toBeVisible();
    await range.press('End');
    await range.blur();
    await expect(
      page.getByRole('button', {
        name: 'I prefer an agenda before accepting a meeting.',
      }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('scroll.png'),
      animations: 'disabled',
    });
    await page.getByRole('button', {name: 'Places'}).click();
    await page.getByRole('button', {name: /^Seal/}).click();
    await expect(page.getByRole('heading', {name: 'The Seal'})).toBeVisible();
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(/Should I keep/);
    const sheet = (await readdir(join(folder, 'desk')))[0];
    expect(await readFile(join(folder, 'desk', sheet), 'utf8')).toContain(
      'Should I keep',
    );
    expect(
      await readFile(join(folder, 'scroll/deciding.md'), 'utf8'),
    ).toContain('I prefer an agenda');
    expect(
      (await readFile(join(folder, '.sumi/moments.db')))
        .subarray(0, 16)
        .toString(),
    ).not.toContain('SQLite format');
  } finally {
    core.stdin.end();
  }
});

test('paper, brush and downloadable local Mac archive', async ({
  page,
}, info) => {
  await page.goto('/website/index.html');
  await expect(page.getByRole('heading', {name: 'Sumi'})).toBeVisible();
  await page.mouse.move(80, 80);
  await page.mouse.down();
  await page.mouse.move(165, 105, {steps: 12});
  await page.mouse.up();
  const painted = await page.locator('canvas').evaluate(el => {
    const canvas = el as HTMLCanvasElement;
    return canvas
      .getContext('2d')!
      .getImageData(0, 0, canvas.width, canvas.height)
      .data.some((n, i) => i % 4 === 3 && n > 0);
  });
  expect(painted).toBe(true);
  const download = page.waitForEvent('download');
  await page.getByRole('link', {name: /Download for Mac/}).click();
  const result = await download;
  expect(result.suggestedFilename()).toBe('Sumi-macOS.zip');
  expect(await result.failure()).toBeNull();
  const bytes = await readFile((await result.path())!);
  expect(bytes.subarray(0, 4).toString('hex')).toBe('504b0304');
  expect(bytes.includes(Buffer.from('Sumi.app/Contents/MacOS/sumi'))).toBe(
    true,
  );
  await page.emulateMedia({colorScheme: 'dark', reducedMotion: 'reduce'});
  await page.setViewportSize({width: 520, height: 480});
  await page.screenshot({
    path: info.outputPath('website-dark.png'),
    animations: 'disabled',
  });
});

test('Mirror separates paired blind results by prediction month and model', async ({
  page,
}) => {
  await page.clock.setFixedTime(new Date(2026, 6, 5, 12));
  await page.addInitScript(() => {
    const cases = [
      ['may-win', '2026-05-20', 'held-out', 'A', 0, 2],
      ['may-both', '2026-05-21', 'held-out', 'A', 2, 3],
      ['may-loss', '2026-05-23', 'held-out', 'B', 3, 0],
      ['may-other-grader', '2026-05-24', 'held-out', 'B', 2, 3],
      ['june-tie', '2026-06-04', 'held-out', 'A', 0, 1],
      ['live', '2026-06-04', 'live', 'A', 3, 3],
      ['unpaired', '2026-06-04', 'held-out', 'A', null, 2],
    ];
    const scores = cases.map(([id, t, scope, model, coldGrade, grade]) => ({
      id,
      predictedAt: t + 'T12:00:00Z',
      date: '2026-07-03T12:00:00Z',
      scope,
      model,
      grader: id === 'may-other-grader' ? 'Z' : undefined,
      coldGrade,
      grade,
      category: 'words',
      reflection: 'reviewed',
    }));
    const state = {
      setup: true,
      current: 'sheet',
      sheet: '',
      sheets: [],
      scores,
      log: [],
      entries: [
        {
          text: 'Sketch before choosing a typeface.',
          updated: '2026-07-04T12:00:00Z',
        },
        {text: 'Use an older idea.', updated: '2026-06-21T12:00:00Z'},
        {text: 'A later entry.', updated: '2026-07-06T12:00:00Z'},
      ],
      routes: [],
      tuning: [],
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: async (cmd: string, p: any) =>
        cmd === 'notebook' && p.operation === 'state' ? state : 1,
    };
  });
  await page.goto('/');
  await expect(page.getByLabel('Desk sheet')).toBeVisible();
  await page.keyboard.press('Meta+3');
  const note = page.getByRole('region', {name: 'Sunday note'});
  await expect(note).toContainText('one new or revised entry');
  await expect(note).toContainText('Sketch before choosing a typeface.');
  await expect(note).toContainText('5 of 7 scored guesses');
  await expect(note).not.toContainText('A later entry');
  await page.getByText('Does personalization help?').click();
  const table = page.getByRole('table', {
    name: 'Matched guesses by prediction month and model',
  });
  await expect(table.getByRole('row')).toHaveCount(5);
  await expect(table.getByRole('row').nth(1).getByRole('cell')).toHaveText([
    '0/1',
    '0/1',
    '0 pp',
  ]);
  await expect(table.getByRole('row').nth(2).getByRole('cell')).toHaveText([
    '1/2',
    '2/2',
    '+50 pp',
  ]);
  await expect(table.getByRole('row').nth(3).getByRole('cell')).toHaveText([
    '1/1',
    '0/1',
    '-100 pp',
  ]);
  await expect(table.getByRole('row').nth(4)).toContainText('Graded by Z');
  await expect(table.getByRole('row').nth(4).getByRole('cell')).toHaveText([
    '1/1',
    '1/1',
    '0 pp',
  ]);
  await expect(table.getByRole('rowheader').first()).toContainText('Jun');
});

test('Desk distinguishes accepted ink, a written alternative, dismissal and moving the caret', async ({
  page,
}, info) => {
  const {core, rpc} = await fixtureNotebook();
  const context = 'I think we can call';
  const hint = page.getByText('accepts the light ink', {exact: false});
  const sheet = page.getByLabel('Desk sheet');
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    const models = await rpc('models');
    const model = models.find((m: any) => m.name === 'llama3.2:3b');
    await rpc('model', model);
    await rpc('finish');
    for (let n = 0; n < 2; ) {
      const id = await rpc('newSheet');
      if (createHash('sha256').update(id).digest()[0] < 52) continue;
      await rpc('save', {
        id,
        text: 'What would you say at the end of this meeting?',
      });
      const sealed = await rpc('sealGuess', {id});
      await rpc('breakSeal', {
        id: sealed.id,
        actual: 'We can call this a wrap today.',
      });
      await rpc('verdict', {id: sealed.id, right: false});
      n++;
    }
    await rpc('newSheet');
    let hold = false;
    let release: (() => void) | undefined;
    await page.exposeFunction('sumiTestIPC', async (op: string, args: any) => {
      const result = await rpc(op, args);
      if (op === 'ghost' && hold)
        await new Promise<void>(r => {
          release = r;
        });
      return result;
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await sheet.evaluate(el => {
      el.addEventListener(
        'input',
        () => {
          const start = performance.now();
          const observer = new MutationObserver(() => {
            if (document.querySelector('.ghost-layer')) {
              el.setAttribute(
                'data-display-ms',
                String(Math.round(performance.now() - start)),
              );
              observer.disconnect();
            }
          });
          observer.observe(document.body, {childList: true, subtree: true});
        },
        {once: true},
      );
    });
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    console.log(
      'Desk input to visible ghost ink (ms)',
      await sheet.getAttribute('data-display-ms'),
    );
    await page.screenshot({
      path: info.outputPath('desk-writing.png'),
      animations: 'disabled',
    });
    await sheet.press('Tab');
    await expect(sheet).toHaveValue(context + ' this a wrap today.');
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    await sheet.press('ArrowLeft');
    await expect(hint).toHaveCount(0);
    await sheet.press('End');
    await expect(hint).toBeVisible();
    await sheet.press('Escape');
    await expect(hint).toHaveCount(0);
    await sheet.fill('');
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    await sheet.pressSequentially(' it a rehearsal.', {delay: 12});
    await sheet.blur();
    await expect
      .poll(
        async () =>
          (await rpc('state')).scores.find((v: any) => v.status === 'edited')
            ?.actual,
      )
      .toBe('it a rehearsal.');
    // Discard an inference that returns after the writer selects earlier text.
    hold = true;
    await sheet.fill(context);
    await expect.poll(() => !!release).toBe(true);
    await sheet.press('ArrowLeft');
    hold = false;
    release!();
    await page.keyboard.press('Meta+3');
    await page.getByText('How the score is measured').click();
    await expect(
      page.getByText('Ghost ink · 1 accepted of 4 shown.'),
    ).toBeVisible();
    await page.getByText('Prediction record', {exact: true}).click();
    await expect(page.getByText('You wrote · it a rehearsal.')).toBeVisible();
    const rows = (await rpc('state')).scores.filter(
      (v: any) => v.category === 'ghost',
    );
    expect(rows.map((r: any) => r.status).sort()).toEqual([
      'accepted',
      'dismissed',
      'edited',
      'shown',
    ]);
    expect(rows.every((r: any) => r.sources.includes('desk'))).toBe(true);
  } finally {
    core.stdin.end();
  }
});

test('morning return preserves the old sheet and delivers a waiting agent its answer', async ({
  page,
}) => {
  page.setDefaultTimeout(12000);
  const {core, rpc, folder} = await fixtureNotebook(true);
  const configPath = join(folder, 'agent-config', '.claude.json');
  const personal = {
    mcpServers: {sumi: {command: 'personal-reader'}},
    theme: 'ink',
  };
  await mkdir(join(folder, 'agent-config'), {recursive: true});
  await writeFile(configPath, JSON.stringify(personal));
  let agent: ReturnType<typeof spawn> | undefined;
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.clock.setFixedTime(new Date(2026, 9, 31, 21));
    await page.goto('/');
    const sheet = page.getByLabel('Desk sheet');
    const evening = 'I want to finish the sketch tomorrow.';
    await sheet.fill(evening);
    await expect(page.getByText('Saved', {exact: true})).toBeVisible();
    await page.keyboard.press('Meta+4');
    await page
      .getByRole('button', {name: 'Claude Code ＋', exact: true})
      .click();
    await page
      .getByRole('button', {name: 'Connect Claude Code', exact: true})
      .click();
    await page
      .locator('summary')
      .filter({hasText: 'Claude Code'})
      .first()
      .click();
    await page
      .getByLabel('Permission', {exact: true})
      .selectOption('Answer for me');
    await expect
      .poll(async () => (await rpc('state')).agents[0]?.permission)
      .toBe('Answer for me');
    await expect(
      page.getByLabel('Line for Claude Code’s instructions'),
    ).toHaveValue(/sumi_answer/);
    await expect(page.getByText('MCP server · sumi-2')).toBeVisible();
    const installed = await readFile(configPath, 'utf8');
    const config = JSON.parse(installed).mcpServers['sumi-2'];
    agent = spawn(config.command, config.args, {
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    const lines = createInterface({input: agent.stdout!})[
      Symbol.asyncIterator
    ]();
    let serial = 0;
    async function ask(name: string, args: Record<string, string | string[]>) {
      agent!.stdin!.write(
        JSON.stringify({
          jsonrpc: '2.0',
          id: ++serial,
          method: 'tools/call',
          params: {name, arguments: args},
        }) + '\n',
      );
      const row = await lines.next();
      if (row.done) throw Error('Agent exited before receiving a result');
      const {result, error} = JSON.parse(row.value);
      if (error) throw Error(error.message);
      if (result.isError) throw Error(result.content[0].text);
      return JSON.parse(result.content[0].text);
    }
    const question = 'Which sketch should I prepare for review?';
    const ticket = await ask('sumi_answer', {question});
    expect(ticket.queued).toBe(true);
    for (let i = 0; i < 64; i++)
      await ask('sumi_context', {task: 'Look up review preferences.'});
    expect((await ask('sumi_answer', {ticket: ticket.id})).queued).toBe(true);
    await page.clock.setFixedTime(new Date(2026, 10, 1, 8));
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect(sheet).toHaveValue('');
    await expect(
      page.getByRole('button', {
        name: /1 question from Claude Code is waiting/,
      }),
    ).toBeVisible();
    await sheet.fill('Start with the ink sketch.');
    await expect(page.getByText('Saved', {exact: true})).toBeVisible();
    await page.reload();
    await expect(sheet).toHaveValue('Start with the ink sketch.');
    await page
      .getByRole('button', {name: /1 question from Claude Code/})
      .click();
    const answerButton = page.getByRole('button', {
      name: 'Answer question',
      exact: true,
    });
    await expect(answerButton).toBeFocused();
    await answerButton.click();
    await page.getByLabel('Your answer').fill('Use the ink sketch.');
    await page.getByRole('button', {name: 'Keep answer'}).click();
    await expect(
      page.getByText('Use the ink sketch.', {exact: true}),
    ).toBeVisible();
    expect(await ask('sumi_answer', {ticket: ticket.id})).toMatchObject({
      answer: 'Use the ink sketch.',
      decidedBy: 'user',
    });
    await page.keyboard.press('Meta+1');
    await expect(
      page.getByRole('button', {name: /is waiting for you/}),
    ).toHaveCount(0);
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: new RegExp(evening)}).click();
    await expect(sheet).toHaveValue(evening);
    await page.reload();
    await expect(sheet).toHaveValue(evening);
    expect((await rpc('state')).sheets).toHaveLength(2);
    await page.keyboard.press('Meta+4');
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await page.keyboard.press('Meta+3');
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
    await page.keyboard.press('Alt+Space');
    await expect(sheet).toHaveValue('');
    expect((await rpc('state')).sheets).toHaveLength(3);
    await test.step('review earned automatic answers and change this agent’s confidence threshold', async () => {
      const request = {
        question: 'Which format do you prefer?',
        options: ['Bullets', 'Prose'],
      };
      let qualified = 0;
      let automatic = 0;
      for (let i = 0; i < 90; i++) {
        const reply = await ask('sumi_answer', request);
        if (!reply.queued) {
          expect(reply.answer).toBe('Bullets');
          automatic++;
          if (reply.confidence >= 0.97 && ++qualified === 20) break;
        }
        await rpc('respond', {id: reply.id, answer: 'Bullets'});
      }
      expect(qualified).toBe(20);
      await page.keyboard.press('Meta+4');
      const agentSection = page
        .locator('details')
        .filter({has: page.getByLabel('Permission', {exact: true})});
      await agentSection.locator('summary').first().click();
      await expect(
        agentSection.getByRole('button', {name: 'Raise minimum to 97%'}),
      ).toHaveCount(0);
      const question = page
        .locator('summary')
        .filter({hasText: request.question})
        .first();
      await question.click();
      await page
        .getByRole('button', {name: 'That was right', exact: true})
        .click();
      await expect(agentSection).toContainText(
        automatic +
          ' of ' +
          automatic +
          ' automatic answers reviewed · 0 corrected.',
      );
      await agentSection
        .getByRole('button', {name: 'Raise minimum to 97%'})
        .click();
      await expect
        .poll(async () => (await rpc('state')).agents[0].threshold)
        .toBe(0.97);
    });
    await writeFile(configPath, 'unfinished json');
    await page.getByRole('button', {name: 'Revoke access'}).click();
    await page.getByRole('button', {name: 'Confirm revocation'}).click();
    await expect(
      page.getByText('Access revoked. Configuration cleanup is pending.'),
    ).toBeVisible();
    await expect(
      ask('sumi_context', {task: 'Check a preference.'}),
    ).rejects.toThrow('not connected');
    await expect(readFile(config.args.at(-1))).rejects.toMatchObject({
      code: 'ENOENT',
    });
    expect(await readFile(configPath, 'utf8')).toBe('unfinished json');
    await writeFile(configPath, installed);
    await page.getByRole('button', {name: 'Finish removal'}).click();
    await expect(
      page.getByRole('button', {name: 'Claude Code ＋'}),
    ).toBeVisible();
    expect(JSON.parse(await readFile(configPath, 'utf8'))).toEqual(personal);
  } finally {
    agent?.stdin?.end();
    core.stdin.end();
  }
});

test('three weeks of observed choices lead to one user-approved revision', async ({
  page,
}) => {
  const text = 'I decline all recurring meetings.';
  const rule = {
    id: 'recurring',
    section: 'deciding',
    authored: true,
    confidence: 1,
    evidence: [],
    sources: [],
  };
  const revision = createHash('sha256').update(text).digest('hex');
  const markdown =
    '# How you decide\n\n## Recurring meetings\n' +
    text +
    '\n<!-- sumi ' +
    JSON.stringify(rule) +
    ' -->\n';
  const scores = [21, 10, 1].map((days, index) => ({
    id: 'case-' + index,
    date: new Date(Date.now() - days * 86400000).toISOString(),
    category: 'decisions',
    scope: 'live',
    grade: 0,
    question: [
      'Join a monthly design review?',
      'Attend the weekly critique?',
      'Accept the recurring studio invite?',
    ][index],
    guess: 'I would decline.',
    actual: [
      'Join the monthly review.',
      'Attend the weekly critique.',
      'Accept the studio invite.',
    ][index],
    ruleIds: ['recurring'],
    ruleRevisions: {recurring: revision},
    ruleCheck: {id: 'recurring', revision},
  }));
  const {rpc, core, folder} = await fixtureNotebook(false, {markdown, scores});
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.keyboard.press('Meta+3');
    await expect(page.getByText(/possible exceptions on 3 days/)).toBeVisible();
    await page.getByText('Read the 3 observations').click();
    await expect(
      page
        .locator('blockquote')
        .filter({hasText: 'Attend the weekly critique.'}),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Change or exception?'}).click();
    const revised = 'I make time for recurring studio critiques.';
    await page
      .getByLabel('How would you word the change or exception?')
      .fill(revised);
    await page.getByRole('button', {name: 'A change', exact: true}).click();
    await expect(page.getByRole('heading', {name: 'Surprises'})).toHaveCount(0);
    await page.keyboard.press('Meta+2');
    await expect(page.getByRole('button', {name: revised})).toBeVisible();
    expect(
      await readFile(join(folder, 'scroll/deciding.md'), 'utf8'),
    ).toContain(revised);
    const state = await rpc('state');
    expect(state.scores.map((v: any) => v.grade)).toEqual([0, 0, 0]);
    expect(
      state.scores.every((v: any) => v.reflection === revised),
    ).toBeTruthy();
  } finally {
    core.stdin.end();
  }
});

test('resume the publisher’s real GGUF download and seal a decision without an external runtime', async ({
  page,
}) => {
  test.skip(
    process.env.SUMI_TEST_MODEL_DOWNLOAD !== '1',
    'Requires the free 1.12 GB publisher download.',
  );
  test.setTimeout(420000);
  page.setDefaultTimeout(15000);
  const first = await fixtureNotebook();
  const partial = join(
    first.folder,
    '.sumi',
    'models',
    'qwen2.5-1.5b-instruct-q4_k_m.gguf.partial',
  );
  let writer: typeof first | undefined, reader: typeof first | undefined;
  const closed = (core: typeof first.core) =>
    new Promise<void>(resolve => {
      if (core.exitCode !== null || core.signalCode !== null) resolve();
      else core.once('exit', () => resolve());
    });
  try {
    await first.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    first.core.stdin.write(
      JSON.stringify({operation: 'downloadBundled', args: {}}) + '\n',
    );
    await expect
      .poll(async () => (await stat(partial).catch(() => null))?.size ?? 0, {
        timeout: 120000,
        intervals: [100],
      })
      .toBeGreaterThan(4_000_000);
    first.core.kill('SIGTERM');
    await closed(first.core);
    const offset = (await stat(partial)).size;
    writer = await fixtureNotebook(false, undefined, first.folder);
    reader = await fixtureNotebook(false, undefined, first.folder);
    const steps: number[] = [];
    await page.exposeFunction('sumiTestIPC', async (op: string, args: any) => {
      if (op === 'models') return []; // Model a Mac with no exposed Ollama or LM Studio.
      if (op === 'modelDownloadStatus') {
        const progress = await reader!.rpc(op, args);
        steps.push(progress.bytes);
        return progress;
      }
      return writer!.rpc(op, args);
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(
      page.getByRole('heading', {name: 'A mind for Sumi.'}),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Resume Qwen 2.5 download'}).click();
    await expect(
      page.getByRole('progressbar', {name: 'Qwen download'}),
    ).toBeVisible();
    await expect(page.getByText('Ready for your notebook.')).toBeVisible({
      timeout: 240000,
    });
    expect(steps).toContain(offset);
    expect(steps.some(n => n > offset)).toBeTruthy();
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await page
      .getByRole('button', {name: 'Continue with a clean page'})
      .click();
    await page.getByRole('button', {name: 'Open the Desk'}).click();
    await page
      .getByLabel('Desk sheet')
      .fill('Should I sketch the cover first or start painting right away?');
    await page.getByRole('button', {name: 'Seal a guess'}).click();
    await page.getByRole('button', {name: 'Open sealed guess'}).click();
    await page
      .getByLabel('Your decision')
      .fill('I will sketch the cover first.');
    await page.getByRole('button', {name: 'Break the seal'}).click();
    const state = await writer.rpc('state');
    expect(state.model.runtime).toBe('Sumi');
    expect(state.sealed.guess.trim().length).toBeGreaterThan(0);
    console.log(
      'Real bundled download resumed at',
      offset,
      'bytes. Model:',
      state.model.name,
    );
  } finally {
    for (const session of [first, reader, writer]) {
      if (!session) continue;
      if (session.core.exitCode === null && session.core.signalCode === null)
        session.core.stdin.end();
      await closed(session.core);
    }
    await rm(first.folder, {recursive: true, force: true});
  }
});
