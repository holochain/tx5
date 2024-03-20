interface AddStatus {
  status: number;
}

type StatusError = Error & AddStatus;

export function err(e: string, s?: number): StatusError {
  const out: any = new Error(e);
  out.status = s || 500;
  return out;
}
