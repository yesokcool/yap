import Foundation

/// Dates as the app writes them (the web's formats).
enum Stamps {
    /// A day divider: "Tuesday, October 6, 2026" (the web's `formatDay`).
    static func day(_ date: Date) -> String {
        date.formatted(.dateTime.weekday(.wide).month(.wide).day().year())
    }

    /// A search result's or pin's time: "Oct 6, 9:41 PM"; the year only
    /// when it isn't this one.
    static func short(_ date: Date, now: Date = .now) -> String {
        if Calendar.current.isDate(date, equalTo: now, toGranularity: .year) {
            return date.formatted(.dateTime.month(.abbreviated).day().hour().minute())
        }
        return date.formatted(.dateTime.month(.abbreviated).day().year().hour().minute())
    }
}
