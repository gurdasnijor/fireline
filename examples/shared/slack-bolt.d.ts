declare module '@slack/bolt' {
  export interface SayMessage {
    readonly text: string
    readonly thread_ts?: string
  }

  export type SayFn = (message: SayMessage) => Promise<unknown>

  export interface AppMentionEvent {
    readonly channel: string
    readonly text: string
    readonly ts: string
    readonly thread_ts?: string
  }

  export interface SlackMessageEvent {
    readonly text?: string
    readonly subtype?: string
    readonly thread_ts?: string
  }

  export class App {
    constructor(options: {
      readonly token: string
      readonly signingSecret: string
      readonly socketMode?: boolean
      readonly appToken?: string
    })

    event(
      name: 'app_mention',
      handler: (args: { readonly event: AppMentionEvent; readonly say: SayFn }) => Promise<void>,
    ): void

    message(
      handler: (args: { readonly message: SlackMessageEvent; readonly say: SayFn }) => Promise<void>,
    ): void

    start(port?: string | number): Promise<void>
  }
}
