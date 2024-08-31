use futures::StreamExt;
use std::convert::TryInto;
use std::error::Error;
use std::fmt;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::codec::{BytesCodec, FramedRead};

const NETWORKMESSAGE_MAXSIZE: usize = 65500;
const INITIAL_BUFFER_POSITION: usize = 8;
const MAX_BODY_LENGTH: usize = NETWORKMESSAGE_MAXSIZE - 2 - 4 - 8;

pub struct NetworkMessage {
    buffer: Vec<u8>,
    position: usize,
    length: usize,
    overrun: bool,
}

impl NetworkMessage {
    pub fn new() -> Self {
        NetworkMessage {
            buffer: vec![0; NETWORKMESSAGE_MAXSIZE],
            position: INITIAL_BUFFER_POSITION,
            length: 0,
            overrun: false,
        }
    }

    pub fn decode_header(&mut self) -> i32 {
        if self.length < 2 {
            println!("Not enough data to decode header");
            return 0;
        }

        let new_size = (self.buffer[0] as i32) | ((self.buffer[1] as i32) << 8);

        if new_size < 0 || new_size as usize > NETWORKMESSAGE_MAXSIZE {
            println!("Invalid decoded header length: {}", new_size);
            return 0;
        }

        self.length = new_size as usize;
        println!("Decoded header length: {}", self.length);
        self.length as i32
    }

    pub fn add_bytes(&mut self, bytes: &[u8]) -> Result<(), NetworkMessageError> {
        if bytes.is_empty() {
            eprintln!("[NetworkMessage::add_bytes] - Bytes is empty");
            return Err(NetworkMessageError::SizeError);
        }
        if !self.can_add(bytes.len()) {
            eprintln!(
                "[NetworkMessage::add_bytes] - NetworkMessage size is wrong: {}",
                bytes.len()
            );
            return Err(NetworkMessageError::SizeError);
        }
        if bytes.len() > NETWORKMESSAGE_MAXSIZE {
            eprintln!(
                "[NetworkMessage::add_bytes] - Exceeded NetworkMessage max size: {}, actual size: {}",
                NETWORKMESSAGE_MAXSIZE, bytes.len()
            );
            return Err(NetworkMessageError::SizeError);
        }

        if self.buffer.len() < self.position + bytes.len() {
            self.buffer.resize(self.position + bytes.len(), 0);
        }

        self.buffer[self.position..self.position + bytes.len()].copy_from_slice(bytes);
        self.position += bytes.len();
        self.length += bytes.len();
        Ok(())
    }

    fn can_read(&self, size: usize) -> bool {
        if (self.position + size) > (self.length + INITIAL_BUFFER_POSITION) || size >= (NETWORKMESSAGE_MAXSIZE - self.position) {
            return false;
        }
        true
    }

    pub fn get_string(&mut self, string_len: Option<usize>) -> Result<String, NetworkMessageError> {
        let string_len = match string_len {
            Some(len) => len,
            None => {
                let len = self.get::<u16>() as usize;
                println!("Comprimento da string lido: {}", len);
                len
            }
        };

        if string_len == 0 {
            println!("O comprimento da string é 0, retornando string vazia.");
            return Ok(String::new());
        }

        if !self.can_read(string_len) {
            self.overrun = true;
            return Err(NetworkMessageError::ReadError);
        }

        let start = self.position;
        self.position += string_len;

        match std::str::from_utf8(&self.buffer[start..self.position]) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => {
                println!("Erro ao decodificar string: {}", e);
                Err(NetworkMessageError::InvalidUtf8)
            }
        }
    }

    pub fn add<T: Copy>(&mut self, value: T) -> Result<(), NetworkMessageError> {
        let size = std::mem::size_of::<T>();

        if !self.can_add(size) {
            return Err(NetworkMessageError::SizeError);
        }

        let value_bytes = unsafe {
            std::slice::from_raw_parts(&value as *const T as *const u8, size)
        };

        self.buffer[self.position..self.position + size].copy_from_slice(value_bytes);
        self.position += size;
        self.length += size;

        Ok(())
    }

    fn get<T>(&mut self) -> T
    where
        T: Copy + Default + Sized,
    {
        let size = std::mem::size_of::<T>();

        if !self.can_read(size) {
            return T::default(); // Retorna o valor padrão para T se não for possível ler
        }

        let mut value: T = T::default();
        let bytes = &self.buffer[self.position..self.position + size];
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                &mut value as *mut T as *mut u8,
                size,
            );
        }

        self.position += size;
        value
    }

    fn can_add(&self, size: usize) -> bool {
        (size + self.position) < MAX_BODY_LENGTH
    }
}

#[derive(Debug)]
pub enum NetworkMessageError {
    SizeError,
    ReadError,
    InvalidUtf8,
}

impl fmt::Display for NetworkMessageError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            NetworkMessageError::SizeError => write!(f, "NetworkMessage size is wrong"),
            NetworkMessageError::ReadError => write!(f, "Cannot read from NetworkMessage"),
            NetworkMessageError::InvalidUtf8 => write!(f, "Invalid UTF-8 string"),
        }
    }
}

impl Error for NetworkMessageError {}

async fn handle_connection(mut inbound: TcpStream, destination: String) -> io::Result<()> {
    let mut outbound = TcpStream::connect(destination).await?;
    let (mut inbound_reader, mut inbound_writer) = inbound.split();
    let (mut outbound_reader, mut outbound_writer) = outbound.split();
    let (tx, mut rx) = mpsc::channel(32);

    let inbound_to_outbound = async {
        let mut framed_read = FramedRead::new(inbound_reader, BytesCodec::new());

        while let Some(Ok(bytes)) = framed_read.next().await {
            println!("Client -> Server Captured: {:?}", &bytes);

            let mut message = NetworkMessage::new();
            if let Err(e) = message.add_bytes(&bytes) {
                eprintln!("Error adding bytes: {}", e);
                continue;
            }

            // Converter os bytes capturados para uma lista de strings hexadecimais
            let decoded_values: Vec<String> = bytes.iter().map(|&byte| format!("{:#x}", byte)).collect();
            println!("Decoded to hex: {:?}", decoded_values);

            match message.get_string(None) {  // None indica que o comprimento da string deve ser lido do buffer
                Ok(s) => println!("String capturada: {}", s),
                Err(e) => println!("Erro ao capturar a string: {}", e),
            }

            tx.send(bytes.to_vec()).await.unwrap();
        }
    };

    let outbound_to_inbound = async {
        let mut framed_read = FramedRead::new(outbound_reader, BytesCodec::new());

        while let Some(Ok(bytes)) = framed_read.next().await {
            inbound_writer.write_all(&bytes).await.unwrap();
        }
    };

    let send_task = async {
        while let Some(buffer) = rx.recv().await {
            outbound_writer.write_all(&buffer).await.unwrap();
        }
    };

    tokio::join!(inbound_to_outbound, outbound_to_inbound, send_task);

    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:7172").await?;

    println!("Listening on 127.0.0.1:7172");

    while let Ok((inbound, _)) = listener.accept().await {
        let destination = "127.0.0.1:7173".to_string();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(inbound, destination).await {
                eprintln!("Error: {}", e);
            }
        });
    }

    Ok(())
}
