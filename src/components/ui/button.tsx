// Originally from Cline (https://github.com/cline/cline) — Copyright Cline Bot Inc. Licensed under Apache-2.0.
// See LICENSES/cline-Apache-2.0.txt. Modified by Netmarble F&C / Engram, 2026.
// Changes: remapped VSCode theme tokens (--vscode-*/button-background/description) to our data-theme
//   tokens (accent/surface/foreground/muted/border); dropped Cline-specific variants (cline, outline-primary,
//   success, danger) unused by the ported chat leaves. Kept default/secondary/error/outline/ghost/link/text/icon.
import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'
import { cn } from '@/lib/utils'

const buttonVariants = cva(
	'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent disabled:cursor-not-allowed disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 cursor-pointer [&_svg]:size-2 overflow-hidden',
	{
		variants: {
			variant: {
				default: 'bg-accent text-white hover:opacity-90',
				secondary: 'bg-surface text-foreground hover:opacity-90 shadow-sm',
				error: 'bg-red-500 text-white hover:bg-red-600 shadow-sm',
				outline: 'hover:bg-accent/10 border border-accent/20 shadow-sm',
				ghost: 'hover:bg-accent/10',
				link: 'text-accent underline-offset-4 hover:underline p-0 m-0 cursor-text select-text',
				text: 'text-foreground cursor-text select-text p-0 m-0',
				icon: 'hover:opacity-80 p-0 m-0 border-0 cursor-pointer hover:shadow-none focus:ring-0 focus:ring-offset-0',
			},
			size: {
				default: 'py-1.5 px-4 [&_svg]:size-3',
				sm: 'py-1 px-3 text-sm [&_svg]:size-2',
				xs: 'p-1 text-xs [&_svg]:size-2',
				lg: 'py-4 px-8 [&_svg]:size-4 font-medium',
				icon: 'px-0.5 m-0 [&_svg]:size-2',
				header: 'py-1 px-4 [&_svg]:size-2.5',
			},
		},
		defaultVariants: {
			variant: 'default',
			size: 'default',
		},
	},
)

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
	asChild?: boolean
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
	({ className, variant, size, asChild = false, ...props }, ref) => {
		const Comp = asChild ? Slot : 'button'
		return <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />
	},
)
Button.displayName = 'Button'

export { Button }
