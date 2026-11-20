use super::codec::{self, DecodeError};
use super::model::{Event, Outcome, BIN_SIZE};
use bytes::{BufMut, Bytes, BytesMut};

#[test]
fn test_event_roundtrip() {
    let outcomes = [Outcome::Unknown, Outcome::Success, Outcome::Failure];

    for (i, outcome) in outcomes.into_iter().enumerate() {
        let original = Event {
            timestamp: 1_700_000_000 + i as i64,
            event_code: 100 + i as u16,
            outcome,
            location_id: 10,
            operation_id: 20,
            actor_id: 1001,
            target_id: 2002,
            payload_id: 3003,
        };

        // 1. Кодируем событие в Bytes
        let encoded = codec::encode(&original);
        assert_eq!(encoded.len(), BIN_SIZE);

        // 2. Декодируем обратно (клонируем Bytes, так как decode сдвигает курсор)
        let mut cursor = encoded.clone();
        let decoded = codec::decode(&mut cursor)
            .expect("Ошибка при декодировании валидного пакета")
            .expect("Пакет должен был прочитаться целиком");

        // 3. Проверяем полное совпадение и пустоту остатка буфера
        assert_eq!(original, decoded);
        assert!(cursor.is_empty(), "После чтения пакета буфер должен быть пустым");

        // 4. Обратный round-trip: повторное кодирование даёт те же байты
        assert_eq!(encoded, codec::encode(&decoded));
    }
}

/// 1. Проверка точного расположения байтов (Little-Endian) и смещений
#[test]
fn test_exact_byte_layout() {
    let event = Event {
        timestamp: 0x0102_0304_0506_0708,
        event_code: 0x0A0B,
        outcome: Outcome::Failure, // 2
        location_id: 0x0C0D,
        operation_id: 0x0E0F,
        actor_id: 0x1112_1314,
        target_id: 0x2122_2324,
        payload_id: 0x3132_3334,
    };

    let encoded = codec::encode(&event);
    assert_eq!(encoded.len(), BIN_SIZE);

    let expected_bytes: [u8; 27] = [
        // timestamp: i64 LE (байты 0..8)
        0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
        // event_code: u16 LE (байты 8..10)
        0x0B, 0x0A,
        // outcome: u8 (байт 10)
        0x02,
        // location_id: u16 LE (байты 11..13)
        0x0D, 0x0C,
        // operation_id: u16 LE (байты 13..15)
        0x0F, 0x0E,
        // actor_id: u32 LE (байты 15..19)
        0x14, 0x13, 0x12, 0x11,
        // target_id: u32 LE (байты 19..23)
        0x24, 0x23, 0x22, 0x21,
        // payload_id: u32 LE (байты 23..27)
        0x34, 0x33, 0x32, 0x31,
    ];

    assert_eq!(&encoded[..], &expected_bytes);
}

/// 2. Проверка граничных значений (MIN / MAX, отрицательный timestamp, нехватка байтов)
#[test]
fn test_boundary_values() {
    let cases = [
        // Минимальные значения (включая отрицательный i64::MIN)
        Event {
            timestamp: i64::MIN,
            event_code: u16::MIN,
            outcome: Outcome::Unknown,
            location_id: u16::MIN,
            operation_id: u16::MIN,
            actor_id: u32::MIN,
            target_id: u32::MIN,
            payload_id: u32::MIN,
        },
        // Максимальные значения
        Event {
            timestamp: i64::MAX,
            event_code: u16::MAX,
            outcome: Outcome::Failure,
            location_id: u16::MAX,
            operation_id: u16::MAX,
            actor_id: u32::MAX,
            target_id: u32::MAX,
            payload_id: u32::MAX,
        },
    ];

    for original in cases {
        let mut encoded = codec::encode(&original);
        let decoded = codec::decode(&mut encoded).unwrap().unwrap();
        assert_eq!(original, decoded);
    }

    // Проверка неполного буфера (26 байтов вместо 27)
    let mut short_buf = Bytes::from(vec![0u8; BIN_SIZE - 1]);
    assert_eq!(codec::decode(&mut short_buf), Ok(None));
    // Буфер не должен был сдвинуться при нехватке данных
    assert_eq!(short_buf.len(), BIN_SIZE - 1);
}

/// 3. Проверка некорректного Outcome
#[test]
fn test_invalid_outcome() {
    let mut raw = vec![0u8; BIN_SIZE];
    // Байт с индексом 10 отвечает за outcome (8 байт timestamp + 2 байта event_code)
    raw[10] = 99;

    let mut bytes = Bytes::from(raw);
    let result = codec::decode(&mut bytes);

    assert_eq!(result, Err(DecodeError::InvalidOutcome(99)));
}

/// 4. Проверка детерминированности (одинаковый результат при повторном вызове)
#[test]
fn test_deterministic_encoding() {
    let event = Event {
        timestamp: 1_700_000_000,
        event_code: 500,
        outcome: Outcome::Success,
        location_id: 1,
        operation_id: 2,
        actor_id: 3,
        target_id: 4,
        payload_id: 5,
    };

    let first_call = codec::encode(&event);
    let second_call = codec::encode(&event);

    assert_eq!(first_call, second_call);
}

/// 5. Последовательное декодирование трёх Event из одного буфера
#[test]
fn test_sequential_decode_three_events() {
    let events = [
        Event {
            timestamp: 100,
            event_code: 1,
            outcome: Outcome::Unknown,
            location_id: 10,
            operation_id: 11,
            actor_id: 1000,
            target_id: 1001,
            payload_id: 1002,
        },
        Event {
            timestamp: 200,
            event_code: 2,
            outcome: Outcome::Success,
            location_id: 20,
            operation_id: 21,
            actor_id: 2000,
            target_id: 2001,
            payload_id: 2002,
        },
        Event {
            timestamp: 300,
            event_code: 3,
            outcome: Outcome::Failure,
            location_id: 30,
            operation_id: 31,
            actor_id: 3000,
            target_id: 3001,
            payload_id: 3002,
        },
    ];

    // Склеиваем все 3 пакета в один поток байтов
    let mut stream_buf = BytesMut::with_capacity(BIN_SIZE * events.len());
    for event in &events {
        stream_buf.put_slice(&codec::encode(event));
    }
    let mut stream = stream_buf.freeze();
    assert_eq!(stream.len(), BIN_SIZE * 3);

    // Читаем их по очереди
    for expected in &events {
        let decoded = codec::decode(&mut stream)
            .expect("Ошибка декодирования в потоке")
            .expect("Ожидался целый пакет");
        assert_eq!(&decoded, expected);
    }

    // После чтения трёх пакетов буфер пуст, а четвёртый вызов возвращает Ok(None)
    assert!(stream.is_empty());
    assert_eq!(codec::decode(&mut stream), Ok(None));
}

/// 6. Неизменность курсора (транзакционность) при InvalidOutcome
#[test]
fn test_cursor_unchanged_on_invalid_outcome() {
    let mut raw = vec![0u8; BIN_SIZE];
    raw[0] = 0xAA; // Контрольный первый байт пакета
    raw[10] = 255; // Недопустимый Outcome (индекс 10)

    let original_bytes = Bytes::from(raw);
    let mut cursor = original_bytes.clone();

    let result = codec::decode(&mut cursor);

    assert_eq!(result, Err(DecodeError::InvalidOutcome(255)));
    // Проверяем, что длина, первый байт и всё содержимое буфера остались нетронутыми
    assert_eq!(cursor.len(), BIN_SIZE);
    assert_eq!(cursor[0], 0xAA);
    assert_eq!(cursor, original_bytes);
}

/// 7. Декодирование заранее заданных Golden Bytes (эталонного массива байтов)
#[test]
fn test_decode_golden_bytes() {
    // Эталонный 27-байтовый кадр, зафиксированный в спецификации протокола
    let golden_bytes: [u8; BIN_SIZE] = [
        // timestamp: 1_700_000_000 (0x0000_0000_6553_F100) в Little-Endian
        0x00, 0xF1, 0x53, 0x65, 0x00, 0x00, 0x00, 0x00,
        // event_code: 42 (0x002A)
        0x2A, 0x00,
        // outcome: Outcome::Success (1)
        0x01,
        // location_id: 512 (0x0200)
        0x00, 0x02,
        // operation_id: 777 (0x0309)
        0x09, 0x03,
        // actor_id: 1_000_000 (0x000F_4240)
        0x40, 0x42, 0x0F, 0x00,
        // target_id: 2_000_000 (0x001E_8480)
        0x80, 0x84, 0x1E, 0x00,
        // payload_id: 0xDEAD_BEEF
        0xEF, 0xBE, 0xAD, 0xDE,
    ];

    let mut bytes = Bytes::copy_from_slice(&golden_bytes);
    let decoded = codec::decode(&mut bytes)
        .expect("Golden bytes должны декодироваться без ошибок")
        .expect("Golden bytes содержат полный пакет");

    let expected = Event {
        timestamp: 1_700_000_000,
        event_code: 42,
        outcome: Outcome::Success,
        location_id: 512,
        operation_id: 777,
        actor_id: 1_000_000,
        target_id: 2_000_000,
        payload_id: 0xDEAD_BEEF,
    };

    assert_eq!(decoded, expected);
    assert!(bytes.is_empty());
}


