// Selora IDE — Premium Custom Icon System
//
// Powered by Iconsax Linear/TwoTone design tokens and bespoke SVGs.
// Strictly NO generic lucide-react or web-slop icons.
// All glyphs follow the 24x24 / 16x16 geometric grid, 1.5px stroke weight,
// and inherit currentColor seamlessly.
import type { ComponentPropsWithoutRef, FC } from 'react';
import {
  Add,
  ArrowUp,
  Stop,
  Paperclip2,
  MagicStar,
  Flash,
  ShieldTick,
  ShieldSecurity,
  LampCharge,
  DocumentText,
  DocumentUpload,
  Code,
  Image,
  TickCircle,
  CloseCircle,
  Key as IconsaxKey,
  Command,
  SearchNormal,
  Global,
  Danger,
  Cpu,
  PresentionChart,
  Eye as IconsaxEye,
  EyeSlash,
  Driver,
  Refresh2,
  ExportSquare,
  Trash,
  ArrowDown2,
  ArrowRight2,
  RowVertical,
  Hierarchy,
  CpuSetting,
  Copy as IconsaxCopy,
  Edit2 as IconsaxEdit,
  RotateLeft as IconsaxRotateLeft,
  DirectSend as IconsaxDirectSend,
  type IconProps as IconsaxIconProps,
} from 'iconsax-react';

export type IconProps = IconsaxIconProps;

// --- 1. Actions & Controls ---
export const Plus: FC<IconProps> = Add;
export const AddIcon: FC<IconProps> = Add;

export const ArrowUpIcon: FC<IconProps> = ArrowUp;
export { ArrowUp };

export const Square: FC<IconProps> = Stop;
export const StopIcon: FC<IconProps> = Stop;

export const X: FC<IconProps> = CloseCircle;
export const CloseIcon: FC<IconProps> = CloseCircle;

export const Check: FC<IconProps> = TickCircle;
export const CheckCircle: FC<IconProps> = TickCircle;
export const XCircle: FC<IconProps> = CloseCircle;

// --- 2. Security & Policy ---
export const ShieldCheck: FC<IconProps> = ShieldTick;
export const ShieldCheckIcon: FC<IconProps> = ShieldTick;

export const ShieldAlert: FC<IconProps> = ShieldSecurity;
export const ShieldAlertIcon: FC<IconProps> = ShieldSecurity;

export const Lightbulb: FC<IconProps> = LampCharge;
export const LightbulbIcon: FC<IconProps> = LampCharge;

export const Key: FC<IconProps> = IconsaxKey;
export const KeyIcon: FC<IconProps> = IconsaxKey;

// --- 3. Navigation & Direction ---
export const ChevronDown: FC<IconProps> = ArrowDown2;
export const ChevronRight: FC<IconProps> = ArrowRight2;

export const Columns: FC<IconProps> = RowVertical;
export const SplitIcon: FC<IconProps> = RowVertical;

export const GitBranch: FC<IconProps> = Hierarchy;
export const GitBranchIcon: FC<IconProps> = Hierarchy;

// --- 4. Content & Attachments ---
export const Paperclip: FC<IconProps> = Paperclip2;
export const PaperclipIcon: FC<IconProps> = Paperclip2;

export const Sparkles: FC<IconProps> = MagicStar;
export const SparklesIcon: FC<IconProps> = MagicStar;

export const Zap: FC<IconProps> = Flash;
export const FlashIcon: FC<IconProps> = Flash;

export const FileText: FC<IconProps> = DocumentText;
export const FileEdit: FC<IconProps> = DocumentText;
export const FilePlus: FC<IconProps> = DocumentUpload;

export const Code2: FC<IconProps> = Code;
export const CodeIcon: FC<IconProps> = Code;

export const ImageIcon: FC<IconProps> = Image;
export { Image };

// --- 5. System, Telemetry & Tools ---
export const Terminal: FC<IconProps> = Command;
export const TerminalIcon: FC<IconProps> = Command;

export const Search: FC<IconProps> = SearchNormal;
export const SearchIcon: FC<IconProps> = SearchNormal;

export const Globe: FC<IconProps> = Global;
export const GlobeIcon: FC<IconProps> = Global;

export const AlertCircle: FC<IconProps> = Danger;
export const DangerIcon: FC<IconProps> = Danger;

export const BrainCircuit: FC<IconProps> = Cpu;
export const CpuIcon: FC<IconProps> = Cpu;

export const BarChart2: FC<IconProps> = PresentionChart;
export const ChartIcon: FC<IconProps> = PresentionChart;

export const Bot: FC<IconProps> = CpuSetting;
export const BotIcon: FC<IconProps> = CpuSetting;

// --- 6. Auth & Gateway ---
export const Eye: FC<IconProps> = IconsaxEye;
export const EyeOff: FC<IconProps> = EyeSlash;

export const Server: FC<IconProps> = Driver;
export const ServerIcon: FC<IconProps> = Driver;

export const Loader2: FC<IconProps> = Refresh2;
export const SpinnerIcon: FC<IconProps> = Refresh2;

export const ExternalLink: FC<IconProps> = ExportSquare;
export const Trash2: FC<IconProps> = Trash;
export const RefreshCw: FC<IconProps> = Refresh2;

// --- 7. Agentic Harness Controls (Copy, Edit, Rewind, Steer, Queue) ---
export const Copy: FC<IconProps> = IconsaxCopy;
export const CopyIcon: FC<IconProps> = IconsaxCopy;

export const Edit: FC<IconProps> = IconsaxEdit;
export const EditIcon: FC<IconProps> = IconsaxEdit;

export const RotateLeft: FC<IconProps> = IconsaxRotateLeft;
export const RewindIcon: FC<IconProps> = IconsaxRotateLeft;

export const DirectSend: FC<IconProps> = IconsaxDirectSend;
export const SteerIcon: FC<IconProps> = IconsaxDirectSend;

