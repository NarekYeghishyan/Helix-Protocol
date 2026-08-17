"use client";

/**
 * Drafting a proposal.
 *
 * The form is not typed out. `proposalActions()` reads the `ProposalAction`
 * variants and their field types out of the IDL, and this renders whatever it
 * finds — so a variant added to the program appears here with inputs of the right
 * shape and no edit to this file.
 *
 * That is not a convenience. `governance/README.md` justifies the closed enum by
 * saying the set of things governance can do is *fixed at deploy time and visible
 * in the IDL*, and a hand-written form would be a second copy of that set — the
 * copy that goes stale, leaving a new variant unreachable with nothing failing.
 *
 * What the form deliberately refuses to do is guess. An `i64` is rendered as
 * seconds with **both** readings shown, because `end_ts` is a moment and
 * `epoch_duration` is a length and the IDL does not distinguish them. Picking one
 * would be right most of the time.
 */

import { PublicKey } from "@solana/web3.js";
import { useMemo, useState } from "react";

import { useFlow } from "@/components/flow";
import { Preview } from "@/components/preview";
import { useExplorer } from "@/components/stake";
import { buildCreateProposal, whyCannotPropose } from "@/lib/actions";
import { shortAddress, toDisplay } from "@/lib/amount";
import type { Fetched, Position, Realm } from "@/lib/chain";
import {
  GOVERNANCE_BOUNDS,
  composeAction,
  describeSeconds,
  proposalActions,
  type ActionField,
} from "@/lib/proposal";

/** One input, shaped by the field's type. */
function Field({
  field,
  value,
  onChange,
  disabled,
}: {
  field: ActionField;
  value: string | boolean;
  onChange: (v: string | boolean) => void;
  disabled: boolean;
}) {
  if (field.input === "boolean") {
    return (
      <label className="check">
        <input
          type="checkbox"
          checked={value === true}
          onChange={(e) => onChange(e.target.checked)}
          disabled={disabled}
        />
        <span className="muted small">{field.name}</span>
      </label>
    );
  }

  const text = typeof value === "string" ? value : "";
  const hint =
    field.input === "seconds"
      ? describeSeconds(text)
      : field.input === "amount"
        ? text && /^\d+$/.test(text)
          ? `${toDisplay(text)} at 9 decimals`
          : null
        : null;

  return (
    <label className="field">
      <span className="muted small">
        {field.name} <span className="mono">{field.type}</span>
      </span>
      <input
        value={text}
        onChange={(e) => onChange(e.target.value)}
        placeholder={
          field.input === "address"
            ? "base58 address"
            : field.input === "amount"
              ? "base units"
              : field.input === "seconds"
                ? "seconds"
                : ""
        }
        spellCheck={false}
        disabled={disabled}
      />
      {/* Both readings of an i64, since the IDL cannot say which was meant. */}
      {hint && <span className="muted small">{hint}</span>}
    </label>
  );
}

export function ProposeForm({
  realm,
  positions,
  proposer,
  reload,
}: {
  realm: Realm;
  positions: Fetched<Position>[];
  proposer: PublicKey;
  reload: () => void;
}) {
  const flow = useFlow(reload);
  const explorer = useExplorer();

  // Derived once. Reading the IDL on every keystroke would be wasteful and, more
  // to the point, would make the field list a moving target while typing.
  const actions = useMemo(() => proposalActions(), []);

  const [open, setOpen] = useState(false);
  const [variantName, setVariantName] = useState(actions[0]?.name ?? "Signal");
  const [values, setValues] = useState<Record<string, string | boolean>>({});
  const [title, setTitle] = useState("");
  const [uri, setUri] = useState("");
  const [positionIndex, setPositionIndex] = useState(0);

  const variant = actions.find((a) => a.name === variantName) ?? actions[0];

  // Only positions that clear the threshold can propose, so the selector shows
  // that rather than offering a choice the program will refuse.
  const eligible = positions.filter(
    (p) => p.account.weighted_amount >= realm.min_weight_to_propose,
  );
  const selected = eligible[positionIndex] ?? eligible[0] ?? null;

  const blocked = whyCannotPropose(realm, selected?.account ?? null, title, uri);

  if (!open) {
    return (
      <div className="row">
        <button onClick={() => setOpen(true)}>Draft a proposal</button>
        <span className="muted small">
          Requires a position holding {toDisplay(realm.min_weight_to_propose.toString())} of
          weight — proposing costs weight, not just rent.
        </span>
      </div>
    );
  }

  const onPreview = () => {
    if (!selected) return;
    flow.preview(() =>
      buildCreateProposal({
        realm,
        proposerPosition: selected.address,
        proposer,
        // Parsing happens inside the builder call, so a bad field becomes a
        // message in the preview panel rather than a thrown render.
        action: composeAction(variant, values),
        title: title.trim(),
        descriptorUri: uri.trim(),
      }),
    );
  };

  const titleBytes = new TextEncoder().encode(title).length;
  const uriBytes = new TextEncoder().encode(uri).length;

  return (
    <div className="proposal">
      <div className="position-head">
        <strong>Draft proposal #{realm.proposal_count.toString()}</strong>
        <button onClick={() => setOpen(false)}>Cancel</button>
      </div>

      {eligible.length === 0 ? (
        <p className="state muted">
          None of your positions holds the{" "}
          {toDisplay(realm.min_weight_to_propose.toString())} of weight this realm requires to
          propose.
        </p>
      ) : (
        <label className="field">
          <span className="muted small">propose with</span>
          <select
            value={positionIndex}
            onChange={(e) => setPositionIndex(Number(e.target.value))}
            disabled={flow.busy}
          >
            {eligible.map((p, i) => (
              <option key={p.address.toBase58()} value={i}>
                #{p.account.position_id.toString()} ·{" "}
                {toDisplay(p.account.weighted_amount.toString())} weight ·{" "}
                {shortAddress(p.address.toBase58())}
              </option>
            ))}
          </select>
          <span className="muted small">
            Proves the threshold only. The position is not consumed and does not vote.
          </span>
        </label>
      )}

      <label className="field">
        <span className="muted small">
          action — {actions.length} the program can perform, and no others
        </span>
        <select
          value={variantName}
          onChange={(e) => {
            setVariantName(e.target.value);
            // Fields are per-variant; carrying values across would silently
            // reuse an address entered for a different purpose.
            setValues({});
          }}
          disabled={flow.busy}
        >
          {actions.map((a) => (
            <option key={a.name} value={a.name}>
              {a.name}
            </option>
          ))}
        </select>
        <span className="muted small">
          A closed set fixed at deploy time — extending it needs a program upgrade, which is
          itself governed. That is why a voter can read the variant and know the blast radius.
        </span>
      </label>

      {variant.fields.length === 0 && (
        <p className="muted small">This action carries no parameters.</p>
      )}

      {variant.fields.map((field) => (
        <Field
          key={field.path}
          field={field}
          value={values[field.path] ?? (field.input === "boolean" ? false : "")}
          onChange={(v) => setValues((prev) => ({ ...prev, [field.path]: v }))}
          disabled={flow.busy}
        />
      ))}

      <label className="field">
        <span className="muted small">
          title — {titleBytes}/{GOVERNANCE_BOUNDS.MAX_TITLE_LEN} bytes
        </span>
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          disabled={flow.busy}
          placeholder="What this proposal does"
        />
      </label>

      <label className="field">
        <span className="muted small">
          link — {uriBytes}/{GOVERNANCE_BOUNDS.MAX_URI_LEN} bytes
        </span>
        <input
          value={uri}
          onChange={(e) => setUri(e.target.value)}
          disabled={flow.busy}
          placeholder="https://… where the full text lives"
          spellCheck={false}
        />
        {/* Bytes, not characters: `title.len()` on a Rust String counts bytes,
            so a limit measured in characters disagrees with the program for
            exactly the users least able to explain why. */}
        <span className="muted small">
          Rationale lives off chain — the program never reads it, and on-chain storage is
          expensive.
        </span>
      </label>

      <div className="row">
        <button className="primary" onClick={onPreview} disabled={flow.busy || blocked !== null}>
          Preview
        </button>
        {blocked && <span className="muted small">{blocked}</span>}
      </div>

      <p className="muted small">
        It is created in <strong>Draft</strong>, not open for voting. The quorum snapshot is
        taken at activation instead, which gives the text a window to be read before the clock
        starts.
      </p>

      <Preview
        state={flow.state}
        onConfirm={flow.confirm}
        onCancel={flow.reset}
        explorerUrl={explorer}
      />
    </div>
  );
}
