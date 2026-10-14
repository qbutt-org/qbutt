// SPDX-License-Identifier: GPL-2.0-or-later

#include <QCoreApplication>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QThread>

int main(int argc, char *argv[])
{
    QCoreApplication app(argc, argv);
    const QStringList arguments = app.arguments().mid(1);
    QFile log(qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_LOG"));
    if (log.open(QIODevice::Append))
    {
        log.write((arguments.join(QLatin1Char(' ')) + QLatin1Char('\n')).toUtf8());
        log.flush();
    }

    const QString fault = qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_FAIL");
    if ((fault == QStringLiteral("hang-version")) || (fault == QStringLiteral("delay-connect")))
    {
        QFile pidFile(qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_PID_FILE"));
        if (!pidFile.open(QIODevice::WriteOnly | QIODevice::Truncate))
            return 1;
        pidFile.write(QByteArray::number(QCoreApplication::applicationPid()));
    }

    if (arguments == QStringList {QStringLiteral("--version-json")})
    {
        if (fault == QStringLiteral("hang-version"))
        {
            QThread::sleep(30);
            return 1;
        }
        if (fault == QStringLiteral("version"))
            return 1;
        const QJsonObject version {{QStringLiteral("product"), QStringLiteral("qcatch")},
            {QStringLiteral("version"), QStringLiteral("1.0.0")},
            {QStringLiteral("protocolVersion"), 1}};
        QFile output;
        if (!output.open(stdout, QIODevice::WriteOnly))
            return 1;
        output.write(QJsonDocument(version).toJson(QJsonDocument::Compact));
        output.write("\n");
        return 0;
    }

    if (arguments.size() == 2 && arguments[0] == QStringLiteral("--connect-browser")
        && (arguments[1] == QStringLiteral("edge") || arguments[1] == QStringLiteral("chrome")))
    {
        if (fault == QStringLiteral("delay-connect"))
        {
            const QString release = qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_RELEASE");
            for (int attempt = 0; attempt < 500 && !QFile::exists(release); ++attempt)
                QThread::msleep(20);
            if (!QFile::exists(release))
                return 1;
        }
        if (fault == QStringLiteral("connect"))
            return 1;
        const QJsonObject result {{QStringLiteral("browser"), arguments[1]},
            {QStringLiteral("extensionDirectory"), qEnvironmentVariable("QBUTT_QCATCH_SETUP_FIXTURE_EXTENSION")},
            {QStringLiteral("extensionId"), QStringLiteral("jnmnnglmejlbjnjmmjampnhfncpcbffh")},
            {QStringLiteral("nativeHost"), QStringLiteral("org.qbutt.qcatch")}};
        QFile output;
        if (!output.open(stdout, QIODevice::WriteOnly))
            return 1;
        output.write(QJsonDocument(result).toJson(QJsonDocument::Compact));
        output.write("\n");
        return 0;
    }
    return 2;
}
