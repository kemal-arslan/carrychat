export function Logo({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 1024 1024" aria-hidden className="logo">
      <rect width="1024" height="1024" rx="224" fill="#14232B" />
      <path
        d="M250 760 C 420 760, 420 512, 512 512 S 604 264, 774 264"
        fill="none"
        stroke="#E9D8B4"
        strokeWidth="56"
        strokeLinecap="round"
        strokeDasharray="4 104"
      />
      <circle cx="250" cy="760" r="84" fill="#E9D8B4" />
      <circle cx="512" cy="512" r="84" fill="#E9D8B4" />
      <circle cx="774" cy="264" r="104" fill="#F2A541" />
    </svg>
  );
}
