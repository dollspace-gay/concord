import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile, unlink, writeFile } from 'node:fs/promises';

const root = new URL('../', import.meta.url);
const require = createRequire(new URL('../concord/web/package.json', import.meta.url));
const Ajv2020 = require('ajv/dist/2020').default;
const addFormats = require('ajv-formats').default;
const ts = require('typescript');
const schema = JSON.parse(await readFile(new URL('concord/web/src/api/generated/contract.schema.json', root)));
const represented = JSON.parse(await readFile(new URL('concord/web/tests/contract-variants.json', root)));
const payloads = JSON.parse(await readFile(new URL('concord/web/tests/contract-payloads.json', root)));

function discriminants(definition) {
  return definition.oneOf.map((variant) => variant.properties?.type?.const).sort();
}

for (const [definition, fixtureKey] of [
  ['ClientMessage', 'client_messages'],
  ['ChatEvent', 'server_events'],
]) {
  const actual = discriminants(schema.$defs[definition]);
  const fixtures = [...represented[fixtureKey]].sort();
  assert.equal(new Set(fixtures).size, fixtures.length, `${fixtureKey} contains duplicates`);
  assert.deepEqual(fixtures, actual, `${fixtureKey} must represent every ${definition} discriminant exactly once`);

  const cases = payloads[fixtureKey];
  assert.equal(cases.length, actual.length, `${fixtureKey} must have one executable payload case per variant`);
  assert.deepEqual(
    cases.map((entry) => entry.minimal.type).sort(),
    actual,
    `${fixtureKey} executable payload tags must cover every ${definition} variant exactly once`,
  );

  const ajv = new Ajv2020({ allErrors: true, strict: true });
  addFormats(ajv);
  ajv.addFormat('int32', { type: 'number', validate: (value) => Number.isInteger(value) && value >= -2147483648 && value <= 2147483647 });
  ajv.addFormat('int64', { type: 'number', validate: Number.isSafeInteger });
  ajv.addFormat('uint', { type: 'number', validate: (value) => Number.isSafeInteger(value) && value >= 0 });
  ajv.addFormat('uint32', { type: 'number', validate: (value) => Number.isInteger(value) && value >= 0 && value <= 4294967295 });
  ajv.addFormat('uint64', { type: 'number', validate: (value) => Number.isSafeInteger(value) && value >= 0 });
  const validate = ajv.compile({ $ref: `#/$defs/${definition}`, $defs: schema.$defs });
  for (const entry of cases) {
    for (const shape of ['minimal', 'edge']) {
      assert.equal(
        validate(entry[shape]),
        true,
        `${definition} ${entry.minimal.type} ${shape} failed the TypeScript runtime schema: ${ajv.errorsText(validate.errors)}`,
      );
    }
  }
}

// Execute the checked-in generated TypeScript validator itself against the
// canonical bytes produced by Rust Serde, rather than validating only the
// pre-Serde authoring fixtures.
const generated = new URL('concord/web/src/api/generated/', root);
const validatorSource = await readFile(new URL('validator.ts', generated), 'utf8');
const runtimeFile = new URL(`validator.contract-check.${process.pid}.mjs`, generated);
const runtimeSource = ts.transpileModule(validatorSource, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText.replace("'ajv/dist/2020'", "'ajv/dist/2020.js'");
await writeFile(runtimeFile, runtimeSource);
try {
  const { isClientMessage, isServerEvent } = await import(`${runtimeFile.href}?v=${Date.now()}`);
  for (const [fixtureKey, validate] of [
    ['client_messages', isClientMessage],
    ['server_events', isServerEvent],
  ]) {
    for (const entry of payloads[fixtureKey]) {
      for (const shape of ['minimal', 'edge']) {
        const canonical = entry[`canonical_${shape}`];
        assert.ok(canonical, `${fixtureKey} ${entry.minimal.type} lacks canonical_${shape} Rust snapshot`);
        assert.equal(validate(canonical), true, `${fixtureKey} ${entry.minimal.type} canonical_${shape} failed generated validator`);
        const wrongTag = structuredClone(canonical);
        wrongTag.type = 'unknown_contract_variant';
        assert.equal(validate(wrongTag), false, `${fixtureKey} ${entry.minimal.type} accepted mutated type`);
        const definition = fixtureKey === 'client_messages' ? 'ClientMessage' : 'ChatEvent';
        const variantSchema = schema.$defs[definition].oneOf.find((candidate) => candidate.properties?.type?.const === canonical.type);
        const required = variantSchema.required?.find((key) => key !== 'type');
        if (required) {
          const wrongField = structuredClone(canonical);
          delete wrongField[required];
          assert.equal(validate(wrongField), false, `${fixtureKey} ${entry.minimal.type} accepted missing ${required}`);
        }
      }
    }
  }

  const messageFixture = payloads.server_events
    .find((entry) => entry.minimal.type === 'message')?.canonical_minimal;
  assert.ok(messageFixture, 'server event corpus must contain a canonical message');
  for (const id of [
    'legacy:not-a-uuid',
    ' historical message id ',
    ` 旧消息:${'界'.repeat(512)} `,
  ]) {
    const historical = structuredClone(messageFixture);
    historical.id = id;
    assert.equal(
      isServerEvent(historical),
      true,
      `generated validator rejected supported historical message ID ${JSON.stringify(id.slice(0, 40))}`,
    );
  }
  for (const id of ['', 'line\nbreak', 'c1\u0085control']) {
    const invalid = structuredClone(messageFixture);
    invalid.id = id;
    assert.equal(isServerEvent(invalid), false, 'generated validator accepted an invalid stored message ID');
  }
} finally {
  await unlink(runtimeFile).catch(() => {});
}

console.log(`contract payload coverage: ${represented.client_messages.length} client + ${represented.server_events.length} server variants, minimal + edge shapes`);
