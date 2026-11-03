// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <functional>
#include <memory>
#include <optional>

#include <QJsonArray>
#include <QObject>
#include <QTimer>
#include <QUrl>

class QWebEnginePage;
class QWebEngineProfile;

class SourceCookies final : public QObject
{
public:
    using Snapshot = std::optional<QJsonArray>;

    explicit SourceCookies(QWebEnginePage *source, QObject *parent = nullptr);
    ~SourceCookies() override;
    void read(const QUrl &url, std::function<void(Snapshot)> completed);
    void apply(const QUrl &url, const QJsonArray &seeds, const QJsonArray &removals,
        std::function<void(bool)> completed);

private:
    void execute(QJsonArray commands, std::function<void(Snapshot)> completed);
    void poll();
    void finish(Snapshot replies);

    QWebEnginePage *m_source;
    std::unique_ptr<QWebEngineProfile> m_profile;
    std::unique_ptr<QWebEnginePage> m_frontend;
    QTimer m_poll;
    QJsonArray m_commands;
    std::function<void(Snapshot)> m_completed;
    bool m_ready = false;
    bool m_sent = false;
    bool m_busy = false;
    bool m_stopping = false;
};
