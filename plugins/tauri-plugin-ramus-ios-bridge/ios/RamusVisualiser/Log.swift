import os

/// Logger for the native visualiser, under the plugin's subsystem.
enum Log {
    static let visualiser = Logger(subsystem: "com.raspsoft.ramus", category: "visualiser")
}
