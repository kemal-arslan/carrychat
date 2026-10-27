import { useState, type ReactNode } from "react";
import { Drawer } from "vaul";
import { useBackStep } from "./back";

/** A dialog as a bottom sheet: dragged down, the scrim, Escape or Android's back button closes it. */
export function Sheet({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  // Closing first plays the sheet's way out, then tells the owner.
  const [open, setOpen] = useState(true);
  useBackStep(open, () => setOpen(false));
  return (
    <Drawer.Root open={open} onOpenChange={setOpen} onAnimationEnd={(isOpen) => !isOpen && onClose()}>
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
