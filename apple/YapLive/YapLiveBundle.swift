import SwiftUI
import WidgetKit

// YapLive: the iPhone's widget extension. For now only the voice call's
// Live Activity (`CallActivityViews` draws it, the app's `LiveCall` runs it).

@main
struct YapLiveBundle: WidgetBundle {
    var body: some Widget {
        CallLiveActivity()
    }
}

struct CallLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: CallActivityAttributes.self) { context in
            CallLockScreen(joinedAt: context.attributes.joinedAt, state: context.state, isStale: context.isStale)
                .activityBackgroundTint(CallDesktop.color(dark: context.state.dark))
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            let state = context.state
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    CallIslandChannel(state: state)
                        .padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    CallIslandClose(joinedAt: context.attributes.joinedAt, state: state, isStale: context.isStale)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    CallIslandPanel(state: state, isStale: context.isStale)
                }
            } compactLeading: {
                CallCompactLeading(state: state)
            } compactTrailing: {
                CallCompactTrailing(state: state)
            } minimal: {
                CallMinimal(state: state)
            }
            .keylineTint(Theme.titleEnd)
        }
    }
}
