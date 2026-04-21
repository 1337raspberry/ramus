type BackHandler = () => boolean;

const stack: BackHandler[] = [];

export function pushBackHandler(fn: BackHandler): () => void {
  stack.push(fn);
  return () => {
    const i = stack.indexOf(fn);
    if (i >= 0) stack.splice(i, 1);
  };
}

export function handleAndroidBack(): boolean {
  for (let i = stack.length - 1; i >= 0; i--) {
    if (stack[i]()) return true;
  }
  return false;
}
