import type { Metadata } from "next";
import type { ReactNode } from "react";

import { WalletShell } from "@/components/Wallet";
import "./globals.css";

export const metadata: Metadata = {
  title: "Helix",
  description: "Analytics and wallet integration for the Helix protocol on Solana.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <WalletShell>{children}</WalletShell>
      </body>
    </html>
  );
}
