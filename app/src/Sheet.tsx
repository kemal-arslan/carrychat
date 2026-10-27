import { useEffect, useRef, useState, type ReactNode } from "react";
import { Drawer } from "vaul";
import { useBackStep } from "./back";

/** Longer than the sheet's way out, which takes half a second. */
const CLOSE_MS = 700;

/** A dialog as a bottom sheet: dragged down, the scrim, Escape or Android's back button closes it. */
export function Sheet({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  // Closing first plays the sheet's way out, then tells the owner, once.
  const [open, setOpen] = useState(true);
  const told = useRef(false);
  const closed = () => {
    if (told.current) return;
    told.current = true;
    onClose();
  };
  useBackStep(open, () => setOpen(false));
  // The way out's end may never be reported, as when the app goes to the
  // background meanwhile; the owner would then think the sheet still open.
  useEffect(() => {
    if (open) return;
    const timer = window.setTimeout(closed, CLOSE_MS);
    return () => window.clearTimeout(timer);
  }, [open]);
  return (
    <Drawer.Root open={open} onOpenChange={setOpen} onAnimationEnd={(isOpen) => !isOpen && closed()}>
      <Drawer.Portal>
        <Drawer.Overlay className="sheet-scrim" />
        <Drawer.Content className="sheet" aria-describedby={undefined}>
          <div className="sheet-grab" aria-hidden />
          <Drawer.Title className="sheet-title">{title}</Drawer.Title>
          <div className="sheet-body">{children}</div>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer.Root>
  );
}
