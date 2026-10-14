// SPDX-License-Identifier: GPL-2.0-or-later
#include <cstdio>

#include <QCoreApplication>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

using namespace Qt::Literals::StringLiterals;

int main(int argc, char *argv[])
{
    QCoreApplication application(argc, argv);
    QFile input;
    if (!input.open(stdin, QIODevice::ReadOnly))
        return 2;
    const auto request = QJsonDocument::fromJson(input.readLine(16 * 1024)).object();
    QFile log(qEnvironmentVariable("QBUTT_SESSION_CHECK_LOG"));
    if (request.isEmpty() || !log.open(QIODevice::Append))
        return 2;
    const QString mode = request.value(u"mode"_s).toString();
    log.write(mode.toUtf8() + '\n');
    log.close();

    QJsonObject response {{u"id"_s, request.value(u"id"_s)}};
    if (mode == u"interactive"_s)
    {
        response.insert(u"session"_s, QJsonObject {
            {u"url"_s, request.value(u"url"_s)},
            {u"userAgent"_s, u"generated manual session"_s},
            {u"cookies"_s, QJsonArray {QJsonObject {
                {u"name"_s, u"bbsession"_s}, {u"value"_s, u"manual"_s},
                {u"domain"_s, u"rutracker.org"_s}, {u"path"_s, u"/forum/"_s},
                {u"secure"_s, true}, {u"httpOnly"_s, true}, {u"hostOnly"_s, true}
            }}}
        });
    }
    else
        response.insert(u"error"_s, QJsonObject {{u"code"_s, u"browser_check_required"_s}});

    const auto bytes = QJsonDocument(response).toJson(QJsonDocument::Compact);
    fwrite(bytes.constData(), 1, bytes.size(), stdout);
    fflush(stdout);
    return 0;
}
