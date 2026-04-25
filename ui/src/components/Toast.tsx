import { useEffect, useState } from "react";
import { create } from "zustand";

interface ToastState {
  message: string | null;
  show: (msg: string) => void;
}

export const useToastStore = create<ToastState>((set) => ({
  message: null,
  show: (msg) => set({ message: msg }),
}));

export default function Toast() {
  const message = useToastStore((s) => s.message);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (!message) return;
    setVisible(true);
    const t = setTimeout(() => {
      setVisible(false);
      setTimeout(() => useToastStore.setState({ message: null }), 300);
    }, 3000);
    return () => clearTimeout(t);
  }, [message]);

  if (!message) return null;

  return <div className={`toast${visible ? " toast-visible" : ""}`}>{message}</div>;
}
