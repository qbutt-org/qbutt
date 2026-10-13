// SPDX-License-Identifier: GPL-2.0-or-later
#include <cstdio>

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTextStream>
#include <QThread>

using namespace Qt::Literals::StringLiterals;

namespace
{
    void send(const QJsonObject &message)
    {
        const QByteArray encoded = QJsonDocument(message).toJson(QJsonDocument::Compact) + '\n';
        std::fwrite(encoded.constData(), 1, encoded.size(), stdout);
        std::fflush(stdout);
    }

    QJsonObject translation(const QJsonObject &prompt)
    {
        QJsonArray blocks;
        QJsonArray terms;
        QJsonArray facts;
        const QString chapter = prompt[u"chapter_id"_s].toString();
        for (const auto &value : prompt[u"blocks"_s].toArray())
        {
            const auto block = value.toObject();
            QJsonArray spans;
            QString source;
            for (const auto &spanValue : block[u"spans"_s].toArray())
            {
                auto span = spanValue.toObject()[u"text"_s].toObject();
                span[u"kind"_s] = u"text"_s;
                source += span[u"text"_s].toString();
                span[u"text"_s] = prompt[u"target_language"_s].toString() + u":"_s
                    + span[u"text"_s].toString();
                span[u"bold"_s] = span[u"bold"_s].toBool();
                span[u"italic"_s] = span[u"italic"_s].toBool();
                span[u"underline"_s] = span[u"underline"_s].toBool();
                span[u"reading"_s] = u""_s;
                span[u"asset_id"_s] = u""_s;
                spans.append(span);
            }
            blocks.append(QJsonObject {{u"id"_s, block[u"id"_s]}, {u"spans"_s, spans}});
            if (chapter == u"1"_s && source.contains(u"Ren is female"_s))
            {
                const QJsonObject evidence {{u"chapter_id"_s, chapter},
                    {u"segment_id"_s, block[u"id"_s]}, {u"quote"_s, source}};
                terms.append(QJsonObject {{u"source"_s, u"Ren"_s}, {u"target"_s, u"Рен"_s},
                    {u"forms"_s, QJsonArray {}}, {u"evidence"_s, evidence}});
                facts.append(QJsonObject {{u"subject"_s, u"Ren"_s}, {u"attribute"_s, u"gender"_s},
                    {u"value"_s, u"female"_s}, {u"evidence"_s, evidence}});
            }
        }
        return {{u"blocks"_s, blocks}, {u"terms"_s, terms}, {u"facts"_s, facts}};
    }
}

int main(int argc, char *argv[])
{
    QCoreApplication application(argc, argv);
    const QDir native(QCoreApplication::applicationDirPath());
    const QString thread = u"text-peer-%1"_s.arg(QCoreApplication::applicationPid());
    QJsonObject features;
    const auto arguments = application.arguments();
    for (qsizetype index = 1; index + 1 < arguments.size(); ++index)
    {
        if (arguments[index] != u"-c"_s || !arguments[index + 1].startsWith(u"features."_s))
            continue;
        const auto option = arguments[index + 1].split(u'=');
        features[option[0].sliced(9)] = option[1] == u"true"_s;
    }

    QString model;
    QTextStream input(stdin);
    while (!input.atEnd())
    {
        const auto request = QJsonDocument::fromJson(input.readLine().toUtf8()).object();
        const QString method = request[u"method"_s].toString();
        const auto id = request[u"id"_s];
        const auto parameters = request[u"params"_s].toObject();
        QJsonObject result;
        if (method == u"initialize"_s)
            result = {};
        else if (method == u"initialized"_s)
            continue;
        else if (method == u"config/read"_s)
            result = {{u"config"_s, QJsonObject {{u"features"_s, features},
                {u"web_search"_s, u"disabled"_s}, {u"notify"_s, QJsonArray {}},
                {u"mcp_servers"_s, QJsonObject {}}}}};
        else if (method == u"account/read"_s)
            result = {{u"account"_s, QJsonObject {{u"type"_s, u"chatgpt"_s}}}};
        else if (method == u"model/list"_s)
        {
            QJsonArray models;
            const QJsonArray efforts {
                QJsonObject {{u"reasoningEffort"_s, u"low"_s}},
                QJsonObject {{u"reasoningEffort"_s, u"medium"_s}}
            };
            for (const QString &name : {u"fixture-text"_s, u"fixture-alt"_s})
            {
                models.append(QJsonObject {{u"model"_s, name},
                    {u"inputModalities"_s, QJsonArray {u"text"_s, u"image"_s}},
                    {u"supportedReasoningEfforts"_s, efforts},
                    {u"defaultReasoningEffort"_s, u"low"_s}, {u"isDefault"_s, name == u"fixture-text"_s}});
            }
            result = {{u"data"_s, models}, {u"nextCursor"_s, QJsonValue::Null}};
        }
        else if (method == u"thread/start"_s)
        {
            model = parameters[u"model"_s].toString();
            if (parameters[u"ephemeral"_s] != true || parameters[u"sandbox"_s] != u"read-only"_s)
                return 2;
            result = {{u"thread"_s, QJsonObject {{u"id"_s, thread},
                {u"ephemeral"_s, true}, {u"path"_s, QJsonValue::Null}}}};
        }
        else if (method == u"turn/start"_s)
        {
            const auto prompt = QJsonDocument::fromJson(parameters[u"input"_s].toArray()[0]
                .toObject()[u"text"_s].toString().toUtf8()).object();
            const QString chapter = prompt[u"chapter_id"_s].toString();
            QFile trace(native.filePath(u"turn-%1.json"_s.arg(QCoreApplication::applicationPid())));
            if (!trace.open(QIODevice::WriteOnly))
                return 3;
            auto recorded = prompt;
            recorded[u"pid"_s] = QCoreApplication::applicationPid();
            recorded[u"model"_s] = model;
            recorded[u"effort"_s] = parameters[u"effort"_s];
            trace.write(QJsonDocument(recorded).toJson());
            trace.close();
            const QJsonObject started {{u"id"_s, u"turn"_s}, {u"status"_s, u"inProgress"_s}};
            send({{u"id"_s, id}, {u"result"_s, QJsonObject {{u"turn"_s, started}}}});
            while (QFile::exists(native.filePath(u"hold-"_s + chapter)))
                QThread::msleep(10);

            const QByteArray translated = QJsonDocument(translation(prompt)).toJson(QJsonDocument::Compact);
            const QJsonObject answer {{u"type"_s, u"agentMessage"_s},
                {u"text"_s, QString::fromUtf8(translated)}};
            const QJsonObject completed {{u"id"_s, u"turn"_s}, {u"status"_s, u"completed"_s},
                {u"items"_s, QJsonArray {answer}}};
            send({{u"method"_s, u"turn/completed"_s},
                {u"params"_s, QJsonObject {{u"threadId"_s, thread}, {u"turn"_s, completed}}}});
            continue;
        }
        else
            return 4;
        send({{u"id"_s, id}, {u"result"_s, result}});
    }
    return 0;
}
