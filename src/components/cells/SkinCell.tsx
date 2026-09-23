import type { SkinInfo } from "../../ipc/types";
import { SKIN_ART_WIDTH } from "../../lib/table";
import { Img, Skeleton } from "../primitives";
import { Tooltip } from "../Tooltip";

const CELL = "flex h-6 items-center justify-center";

/**
 * The default and random skins have no artwork — valorant-api answers with the same "no
 * image" placeholder for every one of them — so those get a word instead of a broken box.
 */
const ARTLESS = /^(Standard|Random)\b/;
const ARTLESS_LABEL: Record<string, string> = { Standard: "Default", Random: "Random" };

/** Equipped weapon skin. Ingame only — the backend sends null everywhere else. */
export function SkinCell({
  skin,
  weapon,
  pending,
}: {
  skin: SkinInfo | null;
  weapon: string;
  pending: boolean;
}) {
  if (!skin) {
    return (
      <div className={CELL}>
        {pending ? (
          <Skeleton className="h-1.5 w-14 rounded-full" />
        ) : (
          <Tooltip content={`No ${weapon} equipped`}>
            <span className="text-[11px] text-neutral-700">—</span>
          </Tooltip>
        )}
      </div>
    );
  }

  const label = skin.name || `${weapon} skin`;
  const artless = ARTLESS.exec(skin.name);

  if (artless) {
    return (
      <Tooltip content={label}>
        <div className={CELL}>
          <span className="text-[10px] text-neutral-600">{ARTLESS_LABEL[artless[1]]}</span>
        </div>
      </Tooltip>
    );
  }

  return (
    <Tooltip content={label}>
      <div className={CELL}>
        <Img
          src={skin.iconUrl}
          alt={label}
          className={`max-h-6 w-full ${SKIN_ART_WIDTH} object-contain`}
          fallback={
            <span className="w-full truncate text-center text-[10px] text-neutral-500">{label}</span>
          }
        />
      </div>
    </Tooltip>
  );
}
